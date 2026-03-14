const SEED_IMAGES = [
  { name: 'Autumn Landscape', path: '/test_images/autumn_landscape.jpg' },
  { name: 'The Scream', path: '/test_images/the_scream.jpg' },
  { name: 'Supremus 55', path: '/test_images/supremus_55.jpg' },
  { name: 'Red Moon', path: '/test_images/red_moon.jpg' },
  { name: 'Milkmaid', path: '/test_images/het_melkmeisje_small.jpg' },
  { name: 'Kodak 01', path: '/test_images/kodim01.png' },
  { name: 'Kodak 23', path: '/test_images/kodim23.png' },
  { name: 'Palette', path: '/test_images/palette.png' },
  { name: 'Ocean', path: '/test_images/ocean.png' },
  { name: 'Sunset', path: '/test_images/sunset.png' },
  { name: 'Forest', path: '/test_images/forest.png' },
  { name: 'Flowers', path: '/test_images/flowers.png' },
];

const dropZone = document.getElementById('drop-zone');
const fileInput = document.getElementById('file-input');
const nInput = document.getElementById('n-input');
const nDisplay = document.getElementById('n-display');
const distInput = document.getElementById('dist-input');
const distDisplay = document.getElementById('dist-display');
const chromaInput = document.getElementById('chroma-input');
const chromaDisplay = document.getElementById('chroma-display');
const maskInput = document.getElementById('mask-input');
const maskDisplay = document.getElementById('mask-display');
const earthInput = document.getElementById('earth-input');
const earthDisplay = document.getElementById('earth-display');
const resultSection = document.getElementById('result-section');
const resultImg = document.getElementById('result-img');
const colorRows = document.getElementById('color-rows');
const errorBox = document.getElementById('error-box');
const toast = document.getElementById('toast');
const uploadTitle = document.getElementById('upload-title');
const uploadSub = document.getElementById('upload-sub');
const colorGroupCanvas = document.getElementById('color-group-canvas');
const imageInfo = document.getElementById('image-info');
const uploadPreviewWrap = document.getElementById('upload-preview-wrap');
const uploadPreview = document.getElementById('upload-preview');
const analyzeStatus = document.getElementById('analyze-status');
const colorGroupCtx = colorGroupCanvas.getContext('2d');
const sourceCanvas = document.createElement('canvas');
const sourceCtx = sourceCanvas.getContext('2d', { willReadFrequently: true });
const fadePrevCanvas = document.createElement('canvas');
const fadePrevCtx = fadePrevCanvas.getContext('2d');
const fadeNextCanvas = document.createElement('canvas');
const fadeNextCtx = fadeNextCanvas.getContext('2d');

let selectedFile = null;
let selectedUrl = null;
let selectedFileObjectUrl = null;
let currentImageId = null;
let selectionVersion = 0;
let uploadPromise = null;
let lastColors = [];
let analyzeTimer = null;
let activeAnalyzeController = null;
let latestRequestId = 0;
let latestCanvasJobId = 0;
let canvasFadeRafId = 0;
let colorGroupCanvasReady = false;
let selectedSeedCard = null;
const COLOR_GROUP_FADE_MS = 180;
const colorGroupWorker = new Worker('color-group.worker.js');
const SETTINGS_STORAGE_KEY = 'colorExtractor.settings.v1';
const CONTROL_IDS = ['n', 'dist', 'chroma', 'mask', 'earth'];
const defaultControlValues = {
  n: nInput.value,
  dist: distInput.value,
  chroma: chromaInput.value,
  mask: maskInput.value,
  earth: earthInput.value,
};

colorGroupWorker.onmessage = (event) => {
  const { jobId, width, height, buffer } = event.data;
  if (jobId !== latestCanvasJobId) return;
  if (!colorGroupCtx) return;
  const pixels = new Uint8ClampedArray(buffer);
  const nextImageData = new ImageData(pixels, width, height);
  drawColorGroupWithFade(colorGroupCtx, nextImageData, width, height);
};

// Info panel toggles
document.querySelectorAll('.info-btn').forEach(btn => {
  btn.addEventListener('click', () => {
    const panel = document.getElementById(btn.dataset.panel);
    const open = panel.classList.toggle('open');
    btn.classList.toggle('active', open);
  });
});

restoreControlValues();
updateControlDisplays();

// Sliders
nInput.addEventListener('input', () => {
  nDisplay.textContent = nInput.value;
  persistControlValues();
  scheduleAnalyze();
});
distInput.addEventListener('input', () => {
  distDisplay.textContent = parseFloat(distInput.value).toFixed(2);
  persistControlValues();
  scheduleAnalyze();
});
chromaInput.addEventListener('input', () => {
  chromaDisplay.textContent = parseFloat(chromaInput.value).toFixed(2);
  persistControlValues();
  scheduleAnalyze();
});
maskInput.addEventListener('input', () => {
  maskDisplay.textContent = parseFloat(maskInput.value).toFixed(1);
  persistControlValues();
  scheduleAnalyze();
});
earthInput.addEventListener('input', () => {
  earthDisplay.textContent = Number(earthInput.value).toFixed(2);
  persistControlValues();
  scheduleAnalyze();
});

// Seed grid
const seedGrid = document.getElementById('seed-grid');
SEED_IMAGES.forEach(({ name, path }) => {
  const card = document.createElement('div');
  card.className = 'seed-card';
  const img = document.createElement('img');
  img.src = path;
  img.alt = name;
  img.onerror = () => { card.style.display = 'none'; };
  const label = document.createElement('div');
  label.className = 'seed-name';
  label.textContent = name;
  card.appendChild(img);
  card.appendChild(label);
  card.addEventListener('click', () => {
    resetSelectionState();
    selectedFile = null;
    selectedUrl = path;
    errorBox.style.display = 'none';
    uploadTitle.textContent = name;
    setUploadPreview(path);
    if (selectedSeedCard) selectedSeedCard.style.outline = '';
    selectedSeedCard = card;
    card.style.outline = '1px solid #fff';
    scheduleAnalyze();
  });
  seedGrid.appendChild(card);
});

// File input
fileInput.addEventListener('change', () => {
  if (fileInput.files[0]) setFile(fileInput.files[0]);
});

function setFile(file) {
  resetSelectionState();
  selectedFile = file;
  selectedUrl = null;
  setSelectedFileObjectUrl(file);
  errorBox.style.display = 'none';
  uploadTitle.textContent = file.name;
  uploadSub.textContent = `${(file.size / 1024).toFixed(1)} KB`;
  setUploadPreview(selectedFileObjectUrl);
  if (selectedSeedCard) {
    selectedSeedCard.style.outline = '';
    selectedSeedCard = null;
  }
  scheduleAnalyze();
}

// Drag and drop
dropZone.addEventListener('dragover', e => { e.preventDefault(); dropZone.classList.add('drag-over'); });
dropZone.addEventListener('dragleave', () => dropZone.classList.remove('drag-over'));
dropZone.addEventListener('drop', e => {
  e.preventDefault();
  dropZone.classList.remove('drag-over');
  const file = e.dataTransfer.files[0];
  if (file && file.type.startsWith('image/')) setFile(file);
});

// Paste
document.addEventListener('paste', (e) => {
  const item = [...e.clipboardData.items].find(i => i.type.startsWith('image/'));
  if (item) setFile(item.getAsFile());
});

function resetSelectionState() {
  selectionVersion += 1;
  currentImageId = null;
  uploadPromise = null;
  if (activeAnalyzeController) activeAnalyzeController.abort();
  clearSelectedFileObjectUrl();
}

function setSelectedFileObjectUrl(file) {
  clearSelectedFileObjectUrl();
  selectedFileObjectUrl = URL.createObjectURL(file);
}

function clearSelectedFileObjectUrl() {
  if (selectedFileObjectUrl) {
    URL.revokeObjectURL(selectedFileObjectUrl);
    selectedFileObjectUrl = null;
  }
}

function setUploadPreview(src) {
  if (!src) {
    uploadPreviewWrap.style.display = 'none';
    uploadPreview.removeAttribute('src');
    return;
  }
  uploadPreview.src = src;
  uploadPreviewWrap.style.display = 'block';
}

function hasSelectedImage() {
  return Boolean(selectedFile || selectedUrl);
}

function setAnalyzeStatus(text) {
  analyzeStatus.textContent = text;
}

function updateControlDisplays() {
  nDisplay.textContent = nInput.value;
  distDisplay.textContent = parseFloat(distInput.value).toFixed(2);
  chromaDisplay.textContent = parseFloat(chromaInput.value).toFixed(2);
  maskDisplay.textContent = parseFloat(maskInput.value).toFixed(1);
  earthDisplay.textContent = Number(earthInput.value).toFixed(2);
}

function sanitizeControlValue(input, rawValue, fallbackValue) {
  const value = Number(rawValue);
  const min = Number(input.min);
  const max = Number(input.max);
  if (!Number.isFinite(value)) return fallbackValue;
  const clamped = Math.min(max, Math.max(min, value));
  const step = Number(input.step);
  if (!Number.isFinite(step) || step <= 0) return String(clamped);
  const snapped = Math.round((clamped - min) / step) * step + min;
  const decimals = (input.step.split('.')[1] || '').length;
  return snapped.toFixed(decimals);
}

function getControlInputs() {
  return {
    n: nInput,
    dist: distInput,
    chroma: chromaInput,
    mask: maskInput,
    earth: earthInput,
  };
}

function persistControlValues() {
  const controls = getControlInputs();
  const payload = {};
  CONTROL_IDS.forEach((id) => {
    payload[id] = controls[id].value;
  });
  localStorage.setItem(SETTINGS_STORAGE_KEY, JSON.stringify(payload));
}

function restoreControlValues() {
  const raw = localStorage.getItem(SETTINGS_STORAGE_KEY);
  if (!raw) return;
  let parsed;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return;
  }
  const controls = getControlInputs();
  CONTROL_IDS.forEach((id) => {
    const input = controls[id];
    input.value = sanitizeControlValue(input, parsed?.[id], defaultControlValues[id]);
  });
}

function scheduleAnalyze() {
  if (!hasSelectedImage()) return;
  if (analyzeTimer) clearTimeout(analyzeTimer);
  analyzeTimer = setTimeout(() => {
    analyzeTimer = null;
    analyze();
  }, 220);
}

async function analyze() {
  if (!hasSelectedImage()) return;

  const n = parseInt(nInput.value, 10) || 5;
  const minDist = parseFloat(distInput.value);
  const chromaWeight = parseFloat(chromaInput.value);
  const maskWeight = parseFloat(maskInput.value);
  const earthBias = parseFloat(earthInput.value);
  const requestId = ++latestRequestId;
  const version = selectionVersion;

  if (activeAnalyzeController) activeAnalyzeController.abort();
  const controller = new AbortController();
  activeAnalyzeController = controller;
  setAnalyzeStatus('Analyzing');
  errorBox.style.display = 'none';

  try {
    let displaySrc;
    if (selectedFile) {
      displaySrc = selectedFileObjectUrl;
    } else if (selectedUrl) {
      displaySrc = selectedUrl;
    } else {
      throw new Error('No image selected');
    }

    const imageId = await ensureImageId(version, controller.signal);
    if (requestId !== latestRequestId) return;

    const query = new URLSearchParams({
      image_id: imageId,
      n: String(n),
      min_dist: String(minDist),
      chroma_weight: String(chromaWeight),
      mask_weight: String(maskWeight),
      earth_bias: String(earthBias),
    });
    const resp = await fetch(`/colors?${query.toString()}`, { method: 'POST', signal: controller.signal });
    const data = await resp.json();

    if (!resp.ok) throw new Error(data.error || `HTTP ${resp.status}`);

    if (requestId !== latestRequestId) return;
    renderResults(data, displaySrc, requestId);
    setAnalyzeStatus('Up to date');
  } catch (err) {
    if (err?.name === 'AbortError') return;
    errorBox.textContent = `Error: ${err.message}`;
    errorBox.style.display = 'block';
    setAnalyzeStatus('Error');
  } finally {
    if (activeAnalyzeController === controller) activeAnalyzeController = null;
  }
}

async function ensureImageId(version, signal) {
  if (version !== selectionVersion) throw new DOMException('Selection changed', 'AbortError');
  if (currentImageId) return currentImageId;
  if (uploadPromise) return uploadPromise;

  uploadPromise = (async () => {
    let blob;
    if (selectedFile) {
      blob = selectedFile;
    } else if (selectedUrl) {
      const res = await fetch(selectedUrl, { signal });
      blob = await res.blob();
    } else {
      throw new Error('No image selected');
    }

    if (version !== selectionVersion) throw new DOMException('Selection changed', 'AbortError');

    const form = new FormData();
    form.append('image', blob, selectedFile ? selectedFile.name : 'image.jpg');

    const resp = await fetch('/images', { method: 'POST', body: form, signal });
    const data = await resp.json();
    if (!resp.ok) throw new Error(data.error || `HTTP ${resp.status}`);
    if (!data.image_id) throw new Error('Missing image_id from server');
    currentImageId = data.image_id;
    return currentImageId;
  })();

  try {
    return await uploadPromise;
  } finally {
    uploadPromise = null;
  }
}

function luminance(hex) {
  const r = parseInt(hex.slice(1,3),16)/255;
  const g = parseInt(hex.slice(3,5),16)/255;
  const b = parseInt(hex.slice(5,7),16)/255;
  return 0.2126*r + 0.7152*g + 0.0722*b;
}

function renderResults(data, imageSrc, requestId) {
  const colors = data.colors || [];
  if (!colors.length) return;

  lastColors = colors;
  resultImg.src = imageSrc;
  const refreshGroups = () => buildColorGroupCanvas(requestId);
  if (resultImg.complete && resultImg.naturalWidth) {
    refreshGroups();
  } else {
    resultImg.addEventListener('load', refreshGroups, { once: true });
  }

  // Image metadata bar
  const info = data.image_info;
  if (info) {
    const csText = info.icc_converted
      ? `${info.color_space} → sRGB`
      : info.icc_present
        ? `${info.color_space} (profile detected, no transform)`
        : info.color_space;
    const csClass = info.icc_converted
      ? 'info-chip converted'
      : info.icc_present
        ? 'info-chip unconverted'
        : 'info-chip';
    imageInfo.replaceChildren();
    const formatChip = document.createElement('span');
    formatChip.className = 'info-chip';
    formatChip.textContent = info.format;
    const separator = document.createElement('span');
    separator.className = 'info-sep';
    separator.textContent = '·';
    const colorChip = document.createElement('span');
    colorChip.className = csClass;
    colorChip.textContent = csText;
    imageInfo.append(formatChip, separator, colorChip);
    imageInfo.style.display = 'flex';
  } else {
    imageInfo.style.display = 'none';
  }

  // Each row i shows a palette of colors[0..i] side by side
  colorRows.innerHTML = colors.map((_, rowIdx) => {
    const palette = colors.slice(0, rowIdx + 1);
    const cells = palette.map(hex => {
      const lum = luminance(hex);
      const fg = lum > 0.35 ? 'rgba(0,0,0,0.8)' : 'rgba(255,255,255,0.9)';
      return `<div class="palette-cell" style="background:${hex};color:${fg}" onclick="copyHex('${hex}')">
        <span class="palette-hex">${hex}</span>
        <span class="palette-copy">copy</span>
      </div>`;
    }).join('');
    return `<div class="palette-row">${cells}</div>`;
  }).join('');

  resultSection.style.display = 'block';
}

function buildColorGroupCanvas(requestId) {
  const w = resultImg.naturalWidth;
  const h = resultImg.naturalHeight;
  if (!w || !h) return;
  if (!sourceCtx) return;

  const sizeChanged = colorGroupCanvas.width !== w || colorGroupCanvas.height !== h;
  if (sizeChanged) {
    colorGroupCanvas.width = w;
    colorGroupCanvas.height = h;
    colorGroupCanvasReady = false;
  }

  sourceCanvas.width = w;
  sourceCanvas.height = h;
  sourceCtx.drawImage(resultImg, 0, 0, w, h);
  const imageData = sourceCtx.getImageData(0, 0, w, h);
  latestCanvasJobId = requestId;
  colorGroupWorker.postMessage(
    {
      jobId: requestId,
      width: w,
      height: h,
      colors: lastColors,
      buffer: imageData.data.buffer,
    },
    [imageData.data.buffer],
  );
}

function drawColorGroupWithFade(ctx, nextImageData, width, height) {
  if (canvasFadeRafId) {
    cancelAnimationFrame(canvasFadeRafId);
    canvasFadeRafId = 0;
  }

  const canFade =
    colorGroupCanvasReady &&
    colorGroupCanvas.width === width &&
    colorGroupCanvas.height === height;

  if (!canFade) {
    ctx.putImageData(nextImageData, 0, 0);
    colorGroupCanvasReady = true;
    return;
  }

  if (!fadePrevCtx || !fadeNextCtx) {
    ctx.putImageData(nextImageData, 0, 0);
    colorGroupCanvasReady = true;
    return;
  }
  fadePrevCanvas.width = width;
  fadePrevCanvas.height = height;
  fadePrevCtx.drawImage(colorGroupCanvas, 0, 0, width, height);

  fadeNextCanvas.width = width;
  fadeNextCanvas.height = height;
  fadeNextCtx.putImageData(nextImageData, 0, 0);

  const start = performance.now();
  const tick = (now) => {
    const t = Math.min(1, (now - start) / COLOR_GROUP_FADE_MS);
    ctx.globalAlpha = 1;
    ctx.drawImage(fadePrevCanvas, 0, 0);
    ctx.globalAlpha = t;
    ctx.drawImage(fadeNextCanvas, 0, 0);
    ctx.globalAlpha = 1;

    if (t < 1) {
      canvasFadeRafId = requestAnimationFrame(tick);
      return;
    }
    canvasFadeRafId = 0;
    colorGroupCanvasReady = true;
  };

  canvasFadeRafId = requestAnimationFrame(tick);
}

function copyHex(hex) {
  navigator.clipboard.writeText(hex).then(() => {
    toast.classList.add('show');
    setTimeout(() => toast.classList.remove('show'), 1200);
  });
}
