mod icc;

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Multipart, Query},
    http::StatusCode,
    response::IntoResponse,
};
use image::ImageReader;
use okmain::{Config, InputImage};
use serde::Deserialize;
use std::io::Cursor;
use tower_http::{cors::CorsLayer, services::ServeDir};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

/// 50 MB upload limit
const MAX_BODY_BYTES: usize = 50 * 1024 * 1024;

#[derive(Debug, Deserialize)]
struct ColorParams {
    #[serde(default = "default_n")]
    n: usize,
    #[serde(default = "default_min_dist")]
    min_dist: f32,
}

fn default_n() -> usize {
    5
}

fn default_min_dist() -> f32 {
    okmain::DEFAULT_ADAPTIVE_MIN_CENTROID_DISTANCE
}

async fn post_colors(
    Query(params): Query<ColorParams>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let n = params.n.max(1);
    let min_dist = params.min_dist.max(0.0);

    // Walk fields; accept the first one named "image" or "file",
    // or the first field that has an image/* content-type.
    let mut image_bytes: Option<Vec<u8>> = None;

    loop {
        match multipart.next_field().await {
            Ok(Some(field)) => {
                let name = field.name().unwrap_or("").to_string();
                let content_type = field.content_type().unwrap_or("").to_string();
                let is_named = name == "image" || name == "file";
                let is_image_ct = content_type.starts_with("image/");

                // Accept fields named "image"/"file" or with an image/* content-type
                if is_named || is_image_ct {
                    match field.bytes().await {
                        Ok(bytes) if !bytes.is_empty() => {
                            image_bytes = Some(bytes.to_vec());
                            if is_named || is_image_ct {
                                break;
                            }
                        }
                        Ok(_) => {}
                        Err(e) => {
                            return (
                                StatusCode::BAD_REQUEST,
                                Json(serde_json::json!({ "error": format!("Failed to read upload: {e}") })),
                            );
                        }
                    }
                }
            }
            Ok(None) => break,
            Err(e) => {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Json(serde_json::json!({ "error": format!("Multipart parse error: {e}") })),
                );
            }
        }
    }

    let bytes = match image_bytes {
        Some(b) => b,
        None => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({ "error": "No image field found in multipart body. Use field name \"image\" or \"file\"." })),
            );
        }
    };

    // Decode the image (keep the format so we can apply ICC correction below).
    let reader = match ImageReader::new(Cursor::new(&bytes)).with_guessed_format() {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("Failed to read image: {e}") })),
            );
        }
    };
    let format = reader.format();
    let img = match reader.decode() {
        Ok(img) => img,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("Failed to decode image: {e}") })),
            );
        }
    };

    // Format label for the response metadata.
    let format_name = match format {
        Some(image::ImageFormat::Jpeg) => "JPEG",
        Some(image::ImageFormat::Png) => "PNG",
        Some(image::ImageFormat::Gif) => "GIF",
        Some(image::ImageFormat::WebP) => "WebP",
        Some(image::ImageFormat::Bmp) => "BMP",
        Some(image::ImageFormat::Tiff) => "TIFF",
        _ => "Unknown",
    };

    // Convert to RGB8.
    let mut rgb_img = img.to_rgb8();

    // Apply ICC color-profile correction (JPEG and PNG).
    //
    // The `image` crate ignores embedded ICC profiles, so cameras/phones that
    // encode images in Display P3 (nearly all iPhones) produce pixel values
    // that are silently misinterpreted as sRGB, collapsing vibrant pinks,
    // greens and yellows into muted brownish tones.  We extract the ICC data
    // from the raw bytes (APP2 for JPEG, iCCP chunk for PNG), compute the
    // linear-light source→sRGB transform matrix, and apply it in-place before
    // colour extraction.
    let icc_data = icc::extract_icc(&bytes);
    let color_space = icc_data
        .as_deref()
        .and_then(icc::profile_description)
        .unwrap_or_else(|| "Untagged".to_string());
    let matrix = icc_data.as_deref().and_then(icc::icc_to_srgb_matrix);
    let icc_converted = matrix.is_some();
    if let Some(m) = matrix {
        icc::apply_matrix_to_rgb8(rgb_img.as_mut(), m);
    }
    let input = match InputImage::try_from(&rgb_img) {
        Ok(i) => i,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("Invalid image dimensions: {e}") })),
            );
        }
    };

    // Run dominant-color extraction (okmain k-means in Oklab space)
    // Pass n directly so the algorithm targets the requested number of clusters.
    let raw_colors = okmain::colors_with_config(input, Config {
        max_colors: n,
        adaptive_min_centroid_distance: min_dist,
        ..Config::default()
    }).expect("default config values should never fail");

    let colors: Vec<String> = raw_colors
        .into_iter()
        .map(|c| format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b))
        .collect();

    let count = colors.len();

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "colors": colors,
            "count": count,
            "image_info": {
                "format": format_name,
                "color_space": color_space,
                "icc_converted": icc_converted,
            },
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

    let app = Router::new()
        .route(
            "/colors",
            axum::routing::post(post_colors)
                // Raise per-route body limit to 50 MB so large photos work
                .layer(DefaultBodyLimit::max(MAX_BODY_BYTES)),
        )
        // Serve test images and the SPA frontend as static files
        .nest_service("/test_images", ServeDir::new("test_images"))
        .fallback_service(ServeDir::new("frontend"))
        .layer(CorsLayer::permissive());

    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();

    tracing::info!("Listening on http://{addr}");
    axum::serve(listener, app).await.unwrap();
}
