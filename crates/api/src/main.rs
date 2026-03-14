mod icc;

use axum::{
    extract::{DefaultBodyLimit, Multipart, Query, State},
    Json, Router,
    http::StatusCode,
    response::IntoResponse,
};
use image::{DynamicImage, ImageDecoder, ImageReader};
use lru::LruCache;
use okmain::{Config, InputImage};
use rgb::RGB8;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    io::Cursor,
    num::NonZeroUsize,
    sync::Arc,
};
use tower_http::{cors::CorsLayer, services::ServeDir};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};
use uuid::Uuid;

const MAX_BODY_BYTES: usize = 50 * 1024 * 1024;
const MAX_STORED_IMAGES: usize = 128;
const ANALYSIS_CACHE_CAPACITY: usize = 4096;

#[derive(Clone)]
struct AppState {
    images: Arc<tokio::sync::Mutex<ImageStore>>,
    analysis_cache: Arc<tokio::sync::Mutex<LruCache<AnalysisCacheKey, Vec<String>>>>,
}

struct ImageStore {
    by_id: HashMap<String, Arc<StoredImage>>,
    order: VecDeque<String>,
}

#[derive(Clone)]
struct StoredImage {
    width: u16,
    height: u16,
    rgb: Vec<u8>,
    image_info: ImageInfo,
}

#[derive(Debug, Clone, Serialize)]
struct ImageInfo {
    format: String,
    color_space: String,
    icc_present: bool,
    icc_converted: bool,
}

#[derive(Debug, Deserialize)]
struct ColorParams {
    image_id: Option<String>,
    #[serde(default = "default_n")]
    n: usize,
    #[serde(default = "default_min_dist")]
    min_dist: f32,
    chroma_weight: Option<f32>,
    mask_weight: Option<f32>,
    earth_bias: Option<f32>,
}

#[derive(Debug, Hash, PartialEq, Eq)]
struct AnalysisCacheKey {
    image_id: String,
    n: usize,
    min_dist_bits: u32,
    chroma_weight_bits: u32,
    mask_weight_bits: u32,
    earth_bias_bits: u32,
}

fn default_n() -> usize {
    5
}

fn default_min_dist() -> f32 {
    okmain::DEFAULT_ADAPTIVE_MIN_CENTROID_DISTANCE
}

#[derive(Debug, Clone)]
struct NormalizedParams {
    n: usize,
    min_dist: f32,
    chroma_weight: f32,
    mask_weight: f32,
    earth_bias: f32,
}

fn normalize_params(params: &ColorParams) -> NormalizedParams {
    NormalizedParams {
        n: params.n.max(1),
        min_dist: params.min_dist.max(0.0),
        chroma_weight: params
            .chroma_weight
            .unwrap_or(okmain::DEFAULT_CHROMA_WEIGHT)
            .clamp(0.0, 1.0),
        mask_weight: params
            .mask_weight
            .unwrap_or(okmain::DEFAULT_MASK_WEIGHT)
            .clamp(0.0, 1.0),
        earth_bias: params.earth_bias.unwrap_or(0.0).clamp(-1.0, 1.0),
    }
}

fn earth_affinity(c: RGB8) -> f32 {
    const EARTH_SWATCHES: &[[u8; 3]] = &[
        [95, 77, 61],
        [122, 97, 68],
        [150, 117, 80],
        [176, 135, 92],
        [119, 103, 67],
        [91, 96, 58],
        [158, 88, 52],
    ];
    let mut best = 0.0f32;
    for sw in EARTH_SWATCHES {
        let dr = c.r as f32 - sw[0] as f32;
        let dg = c.g as f32 - sw[1] as f32;
        let db = c.b as f32 - sw[2] as f32;
        let dist = (dr * dr + dg * dg + db * db).sqrt();
        let affinity = (1.0 - dist / 441.673).clamp(0.0, 1.0);
        if affinity > best {
            best = affinity;
        }
    }
    best
}

fn apply_earth_bias(mut colors: Vec<RGB8>, n: usize, earth_bias: f32) -> Vec<RGB8> {
    if colors.is_empty() {
        return colors;
    }
    if earth_bias.abs() < 0.001 {
        colors.truncate(n);
        return colors;
    }
    let len = colors.len();
    let denom = (len.saturating_sub(1)).max(1) as f32;
    let mut scored: Vec<(f32, usize, RGB8)> = colors
        .into_iter()
        .enumerate()
        .map(|(idx, c)| {
            let base_rank = 1.0 - (idx as f32 / denom);
            let earth = earth_affinity(c);
            let score = base_rank + earth_bias * earth;
            (score, idx, c)
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    scored.into_iter().take(n).map(|(_, _, c)| c).collect()
}

fn error_json(status: StatusCode, message: impl Into<String>) -> (StatusCode, Json<serde_json::Value>) {
    (
        status,
        Json(serde_json::json!({
            "error": message.into()
        })),
    )
}

async fn read_uploaded_image_bytes(mut multipart: Multipart) -> Result<Vec<u8>, (StatusCode, Json<serde_json::Value>)> {
    let mut image_bytes: Option<Vec<u8>> = None;

    loop {
        match multipart.next_field().await {
            Ok(Some(field)) => {
                let name = field.name().unwrap_or("").to_string();
                let content_type = field.content_type().unwrap_or("").to_string();
                let is_named = name == "image" || name == "file";
                let is_image_ct = content_type.starts_with("image/");

                if is_named || is_image_ct {
                    match field.bytes().await {
                        Ok(bytes) if !bytes.is_empty() => {
                            image_bytes = Some(bytes.to_vec());
                            break;
                        }
                        Ok(_) => {}
                        Err(e) => {
                            return Err(error_json(
                                StatusCode::BAD_REQUEST,
                                format!("Failed to read upload: {e}"),
                            ));
                        }
                    }
                }
            }
            Ok(None) => break,
            Err(e) => {
                return Err(error_json(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    format!("Multipart parse error: {e}"),
                ));
            }
        }
    }

    image_bytes.ok_or_else(|| {
        error_json(
            StatusCode::UNPROCESSABLE_ENTITY,
            "No image field found in multipart body. Use field name \"image\" or \"file\".",
        )
    })
}

fn decode_stored_image(bytes: &[u8]) -> Result<StoredImage, (StatusCode, Json<serde_json::Value>)> {
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| error_json(StatusCode::BAD_REQUEST, format!("Failed to read image: {e}")))?;

    let format = reader.format();
    let mut decoder = reader
        .into_decoder()
        .map_err(|e| error_json(StatusCode::BAD_REQUEST, format!("Failed to create decoder: {e}")))?;
    let icc_data = decoder.icc_profile().ok().flatten();
    let img = DynamicImage::from_decoder(decoder)
        .map_err(|e| error_json(StatusCode::BAD_REQUEST, format!("Failed to decode image: {e}")))?;

    let format_name = match format {
        Some(image::ImageFormat::Jpeg) => "JPEG",
        Some(image::ImageFormat::Png) => "PNG",
        Some(image::ImageFormat::Gif) => "GIF",
        Some(image::ImageFormat::WebP) => "WebP",
        Some(image::ImageFormat::Bmp) => "BMP",
        Some(image::ImageFormat::Tiff) => "TIFF",
        _ => "Unknown",
    };

    let mut rgb_img = img.to_rgb8();
    let color_space = icc_data
        .as_deref()
        .and_then(icc::profile_description)
        .unwrap_or_else(|| "Untagged".to_string());
    let icc_present = icc_data.is_some();
    let icc_converted = icc_data
        .as_deref()
        .map(|profile| icc::apply_icc_to_srgb_rgb8(rgb_img.as_mut(), profile))
        .unwrap_or(false);

    let width = u16::try_from(rgb_img.width()).map_err(|_| {
        error_json(
            StatusCode::BAD_REQUEST,
            "Image width exceeds supported limit",
        )
    })?;
    let height = u16::try_from(rgb_img.height()).map_err(|_| {
        error_json(
            StatusCode::BAD_REQUEST,
            "Image height exceeds supported limit",
        )
    })?;

    Ok(StoredImage {
        width,
        height,
        rgb: rgb_img.into_raw(),
        image_info: ImageInfo {
            format: format_name.to_string(),
            color_space,
            icc_present,
            icc_converted,
        },
    })
}

fn run_colors(stored: &StoredImage, params: &NormalizedParams) -> Result<Vec<String>, (StatusCode, Json<serde_json::Value>)> {
    let input = InputImage::from_bytes(stored.width, stored.height, &stored.rgb).map_err(|e| {
        error_json(
            StatusCode::BAD_REQUEST,
            format!("Invalid image dimensions: {e}"),
        )
    })?;

    let candidate_count = if params.earth_bias.abs() > 0.001 {
        usize::min(16, params.n.saturating_add(8))
    } else {
        params.n
    };
    let raw_colors = okmain::colors_with_config(
        input,
        Config {
            max_colors: candidate_count,
            adaptive_min_centroid_distance: params.min_dist,
            chroma_weight: params.chroma_weight,
            mask_weighted_counts_weight: 1.0 - params.chroma_weight,
            mask_weight: params.mask_weight,
            ..Config::default()
        },
    )
    .expect("default config values should never fail");
    let selected = apply_earth_bias(raw_colors, params.n, params.earth_bias);

    Ok(selected
        .into_iter()
        .map(|c| format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b))
        .collect())
}

async fn post_images(
    State(state): State<AppState>,
    multipart: Multipart,
) -> impl IntoResponse {
    let bytes = match read_uploaded_image_bytes(multipart).await {
        Ok(b) => b,
        Err(err) => return err,
    };

    let stored = match decode_stored_image(&bytes) {
        Ok(s) => s,
        Err(err) => return err,
    };

    let image_id = Uuid::new_v4().to_string();
    let image_info = stored.image_info.clone();

    {
        let mut images = state.images.lock().await;
        images.by_id.insert(image_id.clone(), Arc::new(stored));
        images.order.push_back(image_id.clone());
        while images.by_id.len() > MAX_STORED_IMAGES {
            if let Some(old_id) = images.order.pop_front() {
                images.by_id.remove(&old_id);
            } else {
                break;
            }
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "image_id": image_id,
            "image_info": image_info,
        })),
    )
}

async fn post_colors(
    State(state): State<AppState>,
    Query(params): Query<ColorParams>,
) -> impl IntoResponse {
    let image_id = match params.image_id.clone() {
        Some(id) if !id.is_empty() => id,
        _ => return error_json(StatusCode::UNPROCESSABLE_ENTITY, "Missing required query parameter: image_id"),
    };
    let normalized = normalize_params(&params);
    let cache_key = AnalysisCacheKey {
        image_id: image_id.clone(),
        n: normalized.n,
        min_dist_bits: normalized.min_dist.to_bits(),
        chroma_weight_bits: normalized.chroma_weight.to_bits(),
        mask_weight_bits: normalized.mask_weight.to_bits(),
        earth_bias_bits: normalized.earth_bias.to_bits(),
    };

    let stored = {
        let images = state.images.lock().await;
        images.by_id.get(&image_id).cloned()
    };

    let stored = match stored {
        Some(image) => image,
        None => return error_json(StatusCode::NOT_FOUND, "image_id not found. Upload again."),
    };

    if let Some(colors) = {
        let mut cache = state.analysis_cache.lock().await;
        cache.get(&cache_key).cloned()
    } {
        return (
            StatusCode::OK,
            Json(serde_json::json!({
                "colors": colors,
                "count": colors.len(),
                "image_info": stored.image_info,
            })),
        );
    }

    let colors = match run_colors(&stored, &normalized) {
        Ok(c) => c,
        Err(err) => return err,
    };

    {
        let mut cache = state.analysis_cache.lock().await;
        cache.put(cache_key, colors.clone());
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "colors": colors,
            "count": colors.len(),
            "image_info": stored.image_info.clone(),
        })),
    )
}

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3000);

    let state = AppState {
        images: Arc::new(tokio::sync::Mutex::new(ImageStore {
            by_id: HashMap::new(),
            order: VecDeque::new(),
        })),
        analysis_cache: Arc::new(tokio::sync::Mutex::new(LruCache::new(
            NonZeroUsize::new(ANALYSIS_CACHE_CAPACITY).expect("capacity must be non-zero"),
        ))),
    };

    let app = Router::new()
        .route(
            "/images",
            axum::routing::post(post_images)
                .layer(DefaultBodyLimit::max(MAX_BODY_BYTES)),
        )
        .route(
            "/colors",
            axum::routing::post(post_colors)
                .layer(DefaultBodyLimit::max(MAX_BODY_BYTES)),
        )
        .nest_service("/test_images", ServeDir::new("test_images"))
        .fallback_service(ServeDir::new("frontend"))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();

    tracing::info!("Listening on http://{addr}");
    axum::serve(listener, app).await.unwrap();
}
