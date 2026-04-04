import { useState, useEffect, useCallback } from 'preact/hooks';

export type ThemeMode = 'light' | 'dark' | 'system';

function getSystemDark(): boolean {
  return window.matchMedia('(prefers-color-scheme: dark)').matches;
}

function resolveTheme(mode: ThemeMode): boolean {
  if (mode === 'system') return getSystemDark();
  return mode === 'dark';
}

export function useDarkMode(): [boolean, ThemeMode, (mode: ThemeMode) => void] {
  const [mode, setMode] = useState<ThemeMode>(() => {
    const stored = localStorage.getItem('soundsync-theme');
    if (stored === 'light' || stored === 'dark' || stored === 'system') return stored;
    // Migrate old boolean preference
    const oldStored = localStorage.getItem('soundsync-dark-mode');
    if (oldStored === 'true') return 'dark';
    if (oldStored === 'false') return 'light';
    // Default: follow system
    return 'system';
  });

  const [dark, setDark] = useState(() => resolveTheme(mode));

  // Apply theme to DOM and persist
  useEffect(() => {
    const resolved = resolveTheme(mode);
    setDark(resolved);
    document.documentElement.setAttribute('data-theme', resolved ? 'dark' : 'light');
    localStorage.setItem('soundsync-theme', mode);
    // Clean up old key
    localStorage.removeItem('soundsync-dark-mode');
  }, [mode]);

  // Listen for system preference changes when in "system" mode
  useEffect(() => {
    if (mode !== 'system') return;
    const mq = window.matchMedia('(prefers-color-scheme: dark)');
    const handler = (e: MediaQueryListEvent) => {
      setDark(e.matches);
      document.documentElement.setAttribute('data-theme', e.matches ? 'dark' : 'light');
    };
    mq.addEventListener('change', handler);
    return () => mq.removeEventListener('change', handler);
  }, [mode]);

  const setTheme = useCallback((newMode: ThemeMode) => {
    setMode(newMode);
  }, []);

  return [dark, mode, setTheme];
}
