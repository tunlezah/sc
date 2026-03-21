import { useState } from 'preact/hooks';
import * as api from '../../api/rest';

interface LineInProps {
  active: boolean;
  available: boolean;
}

export function LineIn({ active, available }: LineInProps) {
  const [collapsed, setCollapsed] = useState(true);

  const handleToggle = async () => {
    if (active) {
      await api.deactivateLineIn();
    } else {
      await api.activateLineIn();
    }
  };

  return (
    <div class="card">
      <div class="card-header" onClick={() => setCollapsed(!collapsed)}>
        <span class="card-title">
          Line-In {collapsed ? '+' : '-'}
        </span>
      </div>
      <div class={`card-content ${collapsed ? 'collapsed' : ''}`}>
        <div class="line-in-toggle">
          <div>
            <div style={{ fontWeight: 600 }}>Analog Input</div>
            <div style={{ fontSize: '0.8rem', color: 'var(--text-secondary)' }}>
              {available
                ? active
                  ? 'Line-in active. Bluetooth audio disconnected.'
                  : 'Line-in available. Toggle to switch from Bluetooth.'
                : 'No line-in source detected.'}
            </div>
          </div>
          <button
            class={`toggle ${active ? 'active' : ''}`}
            onClick={handleToggle}
            disabled={!available}
          />
        </div>
      </div>
    </div>
  );
}
