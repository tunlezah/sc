import { useAppState } from './hooks/useAppState';
import { useDarkMode } from './hooks/useDarkMode';
import { Header } from './components/Header/Header';
import { DeviceList } from './components/DeviceList/DeviceList';
import { EQControls } from './components/EQControls/EQControls';
import { SpectrumVisualizer } from './components/SpectrumVisualizer/SpectrumVisualizer';
import { MediaControls } from './components/MediaControls/MediaControls';
import { AudioPlayer } from './components/AudioPlayer/AudioPlayer';
import { LineIn } from './components/LineIn/LineIn';

export function App() {
  const { state, spectrum, ws } = useAppState();
  const [dark, toggleDark] = useDarkMode();

  return (
    <div>
      <Header dark={dark} onToggleDark={toggleDark} status={state.status} />

      <SpectrumVisualizer bands={spectrum} dark={dark} />

      <MediaControls
        trackInfo={state.track_info}
        playbackStatus={state.playback_status}
      />

      <AudioPlayer ws={ws} />

      <DeviceList
        devices={state.devices}
        activeDevice={state.active_device}
        status={state.status}
      />

      <EQControls
        bands={state.eq.bands}
        enabled={state.eq.enabled}
      />

      <LineIn active={state.line_in_active} available={state.line_in_available} />
    </div>
  );
}
