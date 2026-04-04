import { VERSION } from '../../version';
import type { ThemeMode } from '../../hooks/useDarkMode';

interface HeaderProps {
  themeMode: ThemeMode;
  onSetTheme: (mode: ThemeMode) => void;
  status: string;
  lineInActive: boolean;
  activeDevice: string | null;
  devices: import('../../types').DeviceInfo[];
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

export function Header({ themeMode, onSetTheme, status, lineInActive, activeDevice, devices }: HeaderProps) {
  // Determine what the active audio input source is
  const activeDeviceInfo = activeDevice ? devices.find(d => d.address === activeDevice) : null;
  const inputLabel = lineInActive
    ? 'Line In'
    : activeDeviceInfo
    ? `BT: ${activeDeviceInfo.name || activeDeviceInfo.address}`
    : null;

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
        {inputLabel && (
          <div class="header-input-indicator">
            <span class="header-input-dot" />
            <span>{inputLabel}</span>
          </div>
        )}
      </div>
      <div class="header-actions">
        <div class="header-status">
          <span class={statusDotClass(status)} />
          <span>{statusLabel(status)}</span>
        </div>
        <div class="theme-switcher">
          <button
            class={`theme-btn ${themeMode === 'light' ? 'active' : ''}`}
            onClick={() => onSetTheme('light')}
            title="Light theme"
          >{'\u2600'}</button>
          <button
            class={`theme-btn ${themeMode === 'system' ? 'active' : ''}`}
            onClick={() => onSetTheme('system')}
            title="System theme"
          >{'\u{1F5A5}'}</button>
          <button
            class={`theme-btn ${themeMode === 'dark' ? 'active' : ''}`}
            onClick={() => onSetTheme('dark')}
            title="Dark theme"
          >{'\u263D'}</button>
        </div>
      </div>
    </header>
  );
}
