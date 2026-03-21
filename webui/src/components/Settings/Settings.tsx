import { useState } from 'preact/hooks';
import * as api from '../../api/rest';

interface SettingsProps {
  deviceName: string;
}

export function Settings({ deviceName }: SettingsProps) {
  const [collapsed, setCollapsed] = useState(true);
  const [name, setName] = useState(deviceName);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);

  const isDirty = name !== deviceName;

  const handleSave = async () => {
    if (!name.trim() || !isDirty) return;
    setSaving(true);
    try {
      await api.setDeviceName(name.trim());
      setSaved(true);
      setTimeout(() => setSaved(false), 2000);
    } finally {
      setSaving(false);
    }
  };

  const handleKeyDown = (e: KeyboardEvent) => {
    if (e.key === 'Enter') handleSave();
  };

  return (
    <div class="card">
      <div class="card-header" onClick={() => setCollapsed(!collapsed)}>
        <span class="card-title">
          Settings {collapsed ? '+' : '-'}
        </span>
      </div>
      <div class={`card-content ${collapsed ? 'collapsed' : ''}`}>
        <div class="settings-field">
          <label class="settings-label" for="device-name">
            A2DP Sink Name
          </label>
          <div style={{ fontSize: '0.8rem', color: 'var(--text-secondary)', marginBottom: '8px' }}>
            The Bluetooth name other devices see when pairing.
          </div>
          <div class="settings-input-row">
            <input
              id="device-name"
              class="settings-input"
              type="text"
              value={name}
              onInput={(e) => setName((e.target as HTMLInputElement).value)}
              onKeyDown={handleKeyDown}
              placeholder="SoundSync"
              maxLength={64}
            />
            <button
              class={`btn btn-primary btn-sm`}
              onClick={handleSave}
              disabled={!isDirty || !name.trim() || saving}
            >
              {saving ? 'Saving...' : saved ? 'Saved' : 'Apply'}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
