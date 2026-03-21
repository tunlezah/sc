import { useRef, useEffect } from 'preact/hooks';

interface SpectrumVisualizerProps {
  bands: number[];
  dark: boolean;
}

export function SpectrumVisualizer({ bands, dark }: SpectrumVisualizerProps) {
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
        // Draw placeholder bars
        const barCount = 64;
        const barWidth = w / barCount - 1;
        ctx.fillStyle = dark ? 'rgba(51, 128, 204, 0.15)' : 'rgba(68, 72, 255, 0.1)';
        for (let i = 0; i < barCount; i++) {
          const barH = Math.random() * 10 + 2;
          ctx.fillRect(i * (barWidth + 1), h - barH, barWidth, barH);
        }
      } else {
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
      }

      animationRef.current = requestAnimationFrame(draw);
    };

    draw();

    return () => cancelAnimationFrame(animationRef.current);
  }, [dark]);

  return (
    <div class="card">
      <div class="spectrum-container">
        <canvas ref={canvasRef} class="spectrum-canvas" />
      </div>
    </div>
  );
}
