import type { AudioCodec } from '../../types';

interface CodecStripProps {
  activeCodec: AudioCodec | null;
}

const CODECS: { id: AudioCodec; label: string; color: string }[] = [
  { id: 'sbc',       label: 'SBC',     color: '#6b7280' },
  { id: 'aac',       label: 'AAC',     color: '#0ea5e9' },
  { id: 'apt_x',     label: 'aptX',    color: '#14b8a6' },
  { id: 'apt_x_hd',  label: 'aptX HD', color: '#6366f1' },
  { id: 'ldac',      label: 'LDAC',    color: '#a855f7' },
];

export function CodecStrip({ activeCodec }: CodecStripProps) {
  return (
    <div class="codec-strip">
      {CODECS.map(({ id, label, color }) => {
        const isActive = activeCodec === id;
        return (
          <div
            key={id}
            class={`codec-chip ${isActive ? 'codec-active' : 'codec-inactive'}`}
            style={isActive ? {
              '--codec-color': color,
              '--codec-bg': color + '1a',
              '--codec-border': color + '66',
              '--codec-glow': color + '33',
            } as any : undefined}
          >
            {isActive && <span class="codec-dot" style={{ background: color }} />}
            <span class="codec-label">{label}</span>
          </div>
        );
      })}
    </div>
  );
}
