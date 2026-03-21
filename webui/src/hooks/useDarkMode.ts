import { useState, useEffect } from 'preact/hooks';

export function useDarkMode(): [boolean, () => void] {
  const [dark, setDark] = useState(() => {
    const stored = localStorage.getItem('soundsync-dark-mode');
    if (stored !== null) return stored === 'true';
    return window.matchMedia('(prefers-color-scheme: dark)').matches;
  });

  useEffect(() => {
    document.documentElement.setAttribute('data-theme', dark ? 'dark' : 'light');
    localStorage.setItem('soundsync-dark-mode', String(dark));
  }, [dark]);

  const toggle = () => setDark((d) => !d);

  return [dark, toggle];
}
