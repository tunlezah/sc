import { useAppState } from './hooks/useAppState';
import { useDarkMode } from './hooks/useDarkMode';
import { Header } from './components/Header/Header';
import { AudioInput } from './components/AudioInput/AudioInput';
import { EQControls } from './components/EQControls/EQControls';
import { SpectrumVisualizer } from './components/SpectrumVisualizer/SpectrumVisualizer';
import { MediaControls } from './components/MediaControls/MediaControls';
import { AudioOutput } from './components/AudioOutput/AudioOutput';
import { Settings } from './components/Settings/Settings';

export function App() {
  const { state, spectrum, ws } = useAppState();
  const [dark, themeMode, setTheme] = useDarkMode();

  return (
    <div class="app-container">
      <Header
        themeMode={themeMode}
        onSetTheme={setTheme}
        status={state.status}
        lineInActive={state.line_in_active}
        activeDevice={state.active_device}
        devices={state.devices}
      />

      <div class="left-column">
        <SpectrumVisualizer bands={spectrum} dark={dark} artworkUrl={state.track_info?.artwork_url} />

        <MediaControls
          trackInfo={state.track_info}
          playbackStatus={state.playback_status}
          ws={ws}
          activeDevice={state.active_device}
        />

        <EQControls
          bands={state.eq.bands}
          enabled={state.eq.enabled}
        />
      </div>

      <div class="right-column">
        <AudioInput
          devices={state.devices}
          activeDevice={state.active_device}
          status={state.status}
          lineInActive={state.line_in_active}
          lineInAvailable={state.line_in_available}
        />

        <AudioOutput
          castDevices={state.cast_devices}
          castActive={state.cast_active}
          airplayDevices={state.airplay_devices}
          airplayActive={state.airplay_active}
        />

        <Settings deviceName={state.device_name} />
      </div>
    </div>
  );
}
