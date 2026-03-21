import { useState, useRef, useCallback } from 'preact/hooks';
import type { EqBand } from '../../types';
import * as api from '../../api/rest';

interface EQControlsProps {
  bands: EqBand[];
  enabled: boolean;
}

function formatFreq(freq: number): string {
  if (freq >= 1000) return `${(freq / 1000).toFixed(freq >= 10000 ? 0 : 1)}k`;
  return `${freq}`;
}

export function EQControls({ bands, enabled }: EQControlsProps) {
  const [collapsed, setCollapsed] = useState(false);
  const [presets, setPresets] = useState<string[]>([]);
  const [presetsLoaded, setPresetsLoaded] = useState(false);
  const debounceTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const loadPresets = async () => {
    if (!presetsLoaded) {
      const list = await api.getPresets();
      setPresets(list);
      setPresetsLoaded(true);
    }
  };

  const handleGainChange = useCallback(
    (index: number, value: number) => {
      if (debounceTimer.current) clearTimeout(debounceTimer.current);
      debounceTimer.current = setTimeout(() => {
        const updates = bands.map((b, i) => ({ gain_db: i === index ? value : b.gain_db }));
        api.updateEq(updates);
      }, 100);
    },
    [bands],
  );

  const handleToggle = () => {
    api.updateEq(bands.map((b) => ({ gain_db: b.gain_db })), !enabled);
  };

  const handlePreset = (name: string) => {
    api.applyPreset(name);
  };

  const handleSavePreset = () => {
    const name = prompt('Preset name:');
    if (name) api.savePreset(name);
  };

  return (
    <div class="card">
      <div class="card-header" onClick={() => { setCollapsed(!collapsed); loadPresets(); }}>
        <span class="card-title">
          Equalizer {collapsed ? '+' : '-'}
        </span>
        <div class="eq-toggle" onClick={(e) => e.stopPropagation()}>
          <span style={{ fontSize: '0.8rem', color: 'var(--text-secondary)' }}>
            {enabled ? 'ON' : 'OFF'}
          </span>
          <button class={`toggle ${enabled ? 'active' : ''}`} onClick={handleToggle} />
        </div>
      </div>
      <div class={`card-content ${collapsed ? 'collapsed' : ''}`}>
        <div class="eq-container">
          <div class="eq-sliders">
            {bands.map((band, i) => (
              <div class="eq-band" key={i}>
                <div class="eq-value">{band.gain_db > 0 ? '+' : ''}{band.gain_db.toFixed(1)}</div>
                <div class="eq-slider-container">
                  <input
                    type="range"
                    class="eq-slider"
                    min="-12"
                    max="12"
                    step="0.5"
                    value={band.gain_db}
                    disabled={!enabled}
                    onInput={(e) => handleGainChange(i, parseFloat((e.target as HTMLInputElement).value))}
                  />
                </div>
                <div class="eq-label">{formatFreq(band.freq)}</div>
              </div>
            ))}
          </div>

          <div class="eq-presets">
            <span style={{ fontSize: '0.8rem', color: 'var(--text-secondary)', marginRight: '4px' }}>Presets:</span>
            {presets.map((name) => (
              <button key={name} class="btn btn-sm btn-secondary" onClick={() => handlePreset(name)}>
                {name}
              </button>
            ))}
            <button class="btn btn-sm btn-primary" onClick={handleSavePreset}>Save</button>
          </div>
        </div>
      </div>
    </div>
  );
}
