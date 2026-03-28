import { useRef, useEffect } from 'preact/hooks';

interface SpectrumVisualizerProps {
  bands: number[];
  dark: boolean;
  artworkUrl?: string;
}

export function SpectrumVisualizer({ bands, dark, artworkUrl }: SpectrumVisualizerProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const animationRef = useRef<number>(0);
  const bandsRef = useRef<number[]>(bands);

  // Keep a ref to latest bands to avoid re-rendering canvas setup
  bandsRef.current = bands;

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    let lastDrawn = false;

    const draw = () => {
      const dpr = window.devicePixelRatio || 1;
      const rect = canvas.getBoundingClientRect();
      canvas.width = rect.width * dpr;
      canvas.height = rect.height * dpr;
      ctx.scale(dpr, dpr);

      const w = rect.width;
      const h = rect.height;
      const b = bandsRef.current;

      // Clear
      ctx.clearRect(0, 0, w, h);

      if (b.length === 0) {
        // No audio data: show a static idle state (not random noise)
        if (!lastDrawn) {
          const barCount = 64;
          const barWidth = w / barCount - 1;
          ctx.fillStyle = dark ? 'rgba(51, 128, 204, 0.1)' : 'rgba(68, 72, 255, 0.07)';
          for (let i = 0; i < barCount; i++) {
            // Static sine-wave pattern instead of random noise
            const barH = 3 + Math.sin(i * 0.2) * 2;
            ctx.fillRect(i * (barWidth + 1), h - barH, barWidth, barH);
          }
          lastDrawn = true;
        }
        // Don't request another frame until we get data - avoid flicker
        animationRef.current = requestAnimationFrame(draw);
        return;
      }

      lastDrawn = false;
      const barCount = b.length;
      const barWidth = w / barCount - 1;

      for (let i = 0; i < barCount; i++) {
        const value = b[i] || 0;
        const barH = value * h;

        // Gradient from primary to secondary
        const gradient = ctx.createLinearGradient(0, h, 0, h - barH);
        if (dark) {
          gradient.addColorStop(0, '#3380CC');
          gradient.addColorStop(1, '#7A44D4');
        } else {
          gradient.addColorStop(0, '#4448FF');
          gradient.addColorStop(1, '#9652F5');
        }

        ctx.fillStyle = gradient;
        ctx.fillRect(i * (barWidth + 1), h - barH, barWidth, barH);
      }

      animationRef.current = requestAnimationFrame(draw);
    };

    draw();

    return () => cancelAnimationFrame(animationRef.current);
  }, [dark]);

  return (
    <div class="card spectrum-card" style={{ position: 'relative', overflow: 'hidden' }}>
      {artworkUrl && (
        <div
          class="spectrum-artwork-bg"
          style={{
            position: 'absolute',
            inset: 0,
            backgroundImage: `url(${artworkUrl})`,
            backgroundSize: 'cover',
            backgroundPosition: 'center',
            filter: 'blur(30px) saturate(0.5)',
            opacity: dark ? 0.08 : 0.06,
            zIndex: 0,
            pointerEvents: 'none',
          }}
        />
      )}
      <div class="spectrum-container" style={{ position: 'relative', zIndex: 1 }}>
        <canvas ref={canvasRef} class="spectrum-canvas" />
      </div>
    </div>
  );
}
