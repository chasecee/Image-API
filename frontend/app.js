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
const analyzeBtn = document.getElementById('analyze-btn');
const nInput = document.getElementById('n-input');
const nDisplay = document.getElementById('n-display');
const distInput = document.getElementById('dist-input');
const distDisplay = document.getElementById('dist-display');
const chromaInput = document.getElementById('chroma-input');
const chromaDisplay = document.getElementById('chroma-display');
const maskInput = document.getElementById('mask-input');
const maskDisplay = document.getElementById('mask-display');
const resultSection = document.getElementById('result-section');
const resultImg = document.getElementById('result-img');
const colorRows = document.getElementById('color-rows');
const errorBox = document.getElementById('error-box');
const toast = document.getElementById('toast');
const uploadTitle = document.getElementById('upload-title');
const uploadSub = document.getElementById('upload-sub');
const colorGroupToggle = document.getElementById('color-group-toggle');
const colorGroupCanvas = document.getElementById('color-group-canvas');
const imageInfo = document.getElementById('image-info');

let selectedFile = null;
let selectedUrl = null;
let lastColors = [];

// Info panel toggles
document.querySelectorAll('.info-btn').forEach(btn => {
  btn.addEventListener('click', () => {
    const panel = document.getElementById(btn.dataset.panel);
    const open = panel.classList.toggle('open');
    btn.classList.toggle('active', open);
  });
});

// Sliders
nInput.addEventListener('input', () => nDisplay.textContent = nInput.value);
distInput.addEventListener('input', () => distDisplay.textContent = parseFloat(distInput.value).toFixed(2));
chromaInput.addEventListener('input', () => chromaDisplay.textContent = parseFloat(chromaInput.value).toFixed(2));
maskInput.addEventListener('input', () => maskDisplay.textContent = parseFloat(maskInput.value).toFixed(1));

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
    selectedFile = null;
    selectedUrl = path;
    analyzeBtn.disabled = false;
    errorBox.style.display = 'none';
    uploadTitle.textContent = name;
    uploadSub.textContent = 'Click Analyze to extract colors';
    document.querySelectorAll('.seed-card').forEach(c => c.style.outline = '');
    card.style.outline = '1px solid #fff';
  });
  seedGrid.appendChild(card);
});

// File input
fileInput.addEventListener('change', () => {
  if (fileInput.files[0]) setFile(fileInput.files[0]);
});

function setFile(file) {
  selectedFile = file;
  selectedUrl = null;
  analyzeBtn.disabled = false;
  errorBox.style.display = 'none';
  uploadTitle.textContent = file.name;
  uploadSub.textContent = `${(file.size / 1024).toFixed(1)} KB`;
  document.querySelectorAll('.seed-card').forEach(c => c.style.outline = '');
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
document.addEventListener('paste', e => {
  const item = [...e.clipboardData.items].find(i => i.type.startsWith('image/'));
  if (item) setFile(item.getAsFile());
});

// Analyze
analyzeBtn.addEventListener('click', analyze);

async function analyze() {
  const n = parseInt(nInput.value) || 5;
  const minDist = parseFloat(distInput.value);
  const chromaWeight = parseFloat(chromaInput.value);
  const maskWeight = parseFloat(maskInput.value);

  analyzeBtn.disabled = true;
  analyzeBtn.innerHTML = '<span class="spinner"></span>Analyzing';
  errorBox.style.display = 'none';

  try {
    let blob;
    if (selectedFile) {
      blob = selectedFile;
      resultImg.src = URL.createObjectURL(selectedFile);
    } else if (selectedUrl) {
      const res = await fetch(selectedUrl);
      blob = await res.blob();
      resultImg.src = selectedUrl;
    } else {
      throw new Error('No image selected');
    }

    const form = new FormData();
    form.append('image', blob, 'image.jpg');

    const url = `/colors?n=${n}&min_dist=${minDist}&chroma_weight=${chromaWeight}&mask_weight=${maskWeight}`;
    const resp = await fetch(url, { method: 'POST', body: form });
    const data = await resp.json();

    if (!resp.ok) throw new Error(data.error || `HTTP ${resp.status}`);

    renderResults(data);
  } catch (err) {
    errorBox.textContent = `Error: ${err.message}`;
    errorBox.style.display = 'block';
  } finally {
    analyzeBtn.disabled = false;
    analyzeBtn.textContent = 'Analyze';
  }
}

function luminance(hex) {
  const r = parseInt(hex.slice(1,3),16)/255;
  const g = parseInt(hex.slice(3,5),16)/255;
  const b = parseInt(hex.slice(5,7),16)/255;
  return 0.2126*r + 0.7152*g + 0.0722*b;
}

function renderResults(data) {
  const colors = data.colors || [];
  if (!colors.length) return;

  lastColors = colors;

  // Reset toggle to off on each new analysis
  colorGroupToggle.checked = false;
  resultImg.style.display = 'block';
  colorGroupCanvas.style.display = 'none';

  // Image metadata bar
  const info = data.image_info;
  if (info) {
    const csText = info.icc_converted
      ? `${info.color_space} → sRGB`
      : info.color_space;
    const csClass = info.icc_converted ? 'info-chip converted' : 'info-chip';
    imageInfo.innerHTML =
      `<span class="info-chip">${info.format}</span>` +
      `<span class="info-sep">·</span>` +
      `<span class="${csClass}">${csText}</span>`;
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
  resultSection.scrollIntoView({ behavior: 'smooth', block: 'start' });
}

// Build a posterized canvas: every pixel replaced by its nearest extracted color
function buildColorGroupCanvas() {
  const rect = resultImg.getBoundingClientRect();
  const w = Math.round(rect.width);
  const h = Math.round(rect.height);
  if (!w || !h) return;

  colorGroupCanvas.width = w;
  colorGroupCanvas.height = h;

  // Reproduce object-fit: cover crop so canvas matches the displayed image exactly
  const iw = resultImg.naturalWidth;
  const ih = resultImg.naturalHeight;
  const scale = Math.max(w / iw, h / ih);
  const sw = w / scale;
  const sh = h / scale;
  const sx = (iw - sw) / 2;
  const sy = (ih - sh) / 2;

  const ctx = colorGroupCanvas.getContext('2d');
  ctx.drawImage(resultImg, sx, sy, sw, sh, 0, 0, w, h);

  // Parse hex colors to [r,g,b] triples
  const rgbColors = lastColors.map(hex => [
    parseInt(hex.slice(1, 3), 16),
    parseInt(hex.slice(3, 5), 16),
    parseInt(hex.slice(5, 7), 16),
  ]);

  const imageData = ctx.getImageData(0, 0, w, h);
  const px = imageData.data;

  for (let i = 0; i < px.length; i += 4) {
    const r = px[i], g = px[i + 1], b = px[i + 2];
    let minDist = Infinity, nr = r, ng = g, nb = b;
    for (const [cr, cg, cb] of rgbColors) {
      const d = (r - cr) ** 2 + (g - cg) ** 2 + (b - cb) ** 2;
      if (d < minDist) { minDist = d; nr = cr; ng = cg; nb = cb; }
    }
    px[i] = nr; px[i + 1] = ng; px[i + 2] = nb;
  }

  ctx.putImageData(imageData, 0, 0);
}

colorGroupToggle.addEventListener('change', () => {
  if (colorGroupToggle.checked) {
    const apply = () => {
      buildColorGroupCanvas();
      resultImg.style.display = 'none';
      colorGroupCanvas.style.display = 'block';
    };
    if (resultImg.complete && resultImg.naturalWidth) {
      apply();
    } else {
      resultImg.addEventListener('load', apply, { once: true });
    }
  } else {
    resultImg.style.display = 'block';
    colorGroupCanvas.style.display = 'none';
  }
});

function copyHex(hex) {
  navigator.clipboard.writeText(hex).then(() => {
    toast.classList.add('show');
    setTimeout(() => toast.classList.remove('show'), 1200);
  });
}
