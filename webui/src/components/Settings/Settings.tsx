import { useState } from 'preact/hooks';
import * as api from '../../api/rest';

interface SettingsProps {
  deviceName: string;
}

export function Settings({ deviceName }: SettingsProps) {
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
    <div class="settings-row">
      <label class="settings-label" for="device-name">Device Name</label>
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
        class="btn btn-primary btn-sm"
        onClick={handleSave}
        disabled={!isDirty || !name.trim() || saving}
      >
        {saving ? '...' : saved ? 'Saved' : 'Apply'}
      </button>
    </div>
  );
}
