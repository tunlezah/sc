import { VERSION } from '../../version';
import * as api from '../../api/rest';

interface HeaderProps {
  dark: boolean;
  onToggleDark: () => void;
  status: string;
  lineInActive: boolean;
  lineInAvailable: boolean;
}

function statusLabel(status: string): string {
  switch (status) {
    case 'scanning': return 'Bluetooth Scanning';
    case 'ready': return 'Bluetooth Ready';
    case 'unavailable': return 'Bluetooth Unavailable';
    case 'connecting': return 'Connecting';
    default:
      if (status.startsWith('error')) return 'Bluetooth Error';
      return status;
  }
}

function statusDotClass(status: string): string {
  if (status === 'scanning') return 'header-status-dot scanning';
  if (status === 'ready') return 'header-status-dot ready';
  return 'header-status-dot';
}

export function Header({ dark, onToggleDark, status, lineInActive, lineInAvailable }: HeaderProps) {
  const handleLineInToggle = async () => {
    if (lineInActive) {
      await api.deactivateLineIn();
    } else {
      await api.activateLineIn();
    }
  };

  return (
    <header class="header">
      <div class="header-left">
        <img src="/SoundSyncLogo.png" alt="SoundSync" class="header-logo" />
        <div>
          <span class="header-title">SoundSync</span>
          <span class="header-version"> {VERSION}</span>
        </div>
      </div>
      <div class="header-center">
        {lineInAvailable && (
          <div class="header-line-in">
            <span>Line-In</span>
            <button
              class={`toggle toggle-sm ${lineInActive ? 'active' : ''}`}
              onClick={handleLineInToggle}
            />
          </div>
        )}
      </div>
      <div class="header-actions">
        <div class="header-status">
          <span class={statusDotClass(status)} />
          <span>{statusLabel(status)}</span>
        </div>
        <button class="btn-icon" onClick={onToggleDark} title="Toggle dark mode">
          {dark ? '\u2600' : '\u263D'}
        </button>
      </div>
    </header>
  );
}
