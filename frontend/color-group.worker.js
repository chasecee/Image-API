function hexToRgb(hex) {
  return [
    parseInt(hex.slice(1, 3), 16),
    parseInt(hex.slice(3, 5), 16),
    parseInt(hex.slice(5, 7), 16),
  ];
}

self.onmessage = (event) => {
  const { jobId, width, height, colors, buffer } = event.data;
  const px = new Uint8ClampedArray(buffer);
  const rgbColors = colors.map(hexToRgb);

  for (let i = 0; i < px.length; i += 4) {
    const r = px[i];
    const g = px[i + 1];
    const b = px[i + 2];
    let minDist = Infinity;
    let nr = r;
    let ng = g;
    let nb = b;

    for (let j = 0; j < rgbColors.length; j += 1) {
      const cr = rgbColors[j][0];
      const cg = rgbColors[j][1];
      const cb = rgbColors[j][2];
      const d = (r - cr) ** 2 + (g - cg) ** 2 + (b - cb) ** 2;
      if (d < minDist) {
        minDist = d;
        nr = cr;
        ng = cg;
        nb = cb;
      }
    }

    px[i] = nr;
    px[i + 1] = ng;
    px[i + 2] = nb;
  }

  self.postMessage(
    {
      jobId,
      width,
      height,
      buffer: px.buffer,
    },
    [px.buffer],
  );
};
