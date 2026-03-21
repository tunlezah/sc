import { VERSION } from '../../version';

interface HeaderProps {
  dark: boolean;
  onToggleDark: () => void;
  status: string;
}

export function Header({ dark, onToggleDark, status }: HeaderProps) {
  return (
    <header class="header">
      <div class="header-left">
        <img src="/StreamCastImage.png" alt="SoundSync" class="header-logo" />
        <div>
          <span class="header-title">SoundSync</span>
          <span class="header-version"> {VERSION}</span>
        </div>
      </div>
      <div class="header-actions">
        <span style={{ fontSize: '0.8rem', color: 'var(--text-secondary)' }}>
          {status === 'scanning' ? 'Scanning...' : status}
        </span>
        <button class="btn-icon" onClick={onToggleDark} title="Toggle dark mode">
          {dark ? '\u2600' : '\u263D'}
        </button>
      </div>
    </header>
  );
}
