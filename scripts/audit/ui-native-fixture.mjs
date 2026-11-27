// Local browser-only IPC fixtures. They never call a native app or read user data.
import { mockIPC, mockWindows } from '@tauri-apps/api/mocks';

export async function installNativeAudit(api, calls) {
  if (!window.__auditSettings) throw new Error('Native audit requires captured App settings');
  mockIPC((command, args) => {
    calls.push(['native', command, args]);
    if (command === 'plugin:window|inner_size' || command === 'plugin:window|outer_size') return { width: 1280, height: 720 };
    if (command === 'plugin:window|inner_position' || command === 'plugin:window|outer_position') return { x: 0, y: 0 };
    if (command === 'plugin:window|scale_factor') return 1;
    if (command.startsWith('plugin:window|')) return false;
    if (command === 'plugin:notification|register_listener') return 1;
    return null;
  }, { shouldMockEvents: true });
  mockWindows('main');
  api.settings = async () => ({ ...window.__auditSettings, auto_check_app_update: false });
  api.isSilentStartup = async () => false;
  api.tagList = async () => [];
  api.taskTagsListAll = async () => ({});
  api.savedViewList = async () => [];
  api.powerActionState = async () => ({ action: 'none', phase: 'idle', remaining_seconds: 0, target_count: 0 });
}
