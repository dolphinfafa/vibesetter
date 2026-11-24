import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { defaultRoute, defaultSettings, type Assistant, type SessionEvent } from './types';

export const isDesktop = '__TAURI_INTERNALS__' in window;
const previewKey = 'vibesetter.preview.assistants';

// Preview contains real editable local assistants, never simulated devices or credentials.
export async function command<T>(name: string, args: Record<string, unknown> = {}): Promise<T> {
  if (isDesktop) return invoke<T>(name, args);
  const assistants = (): Assistant[] => JSON.parse(localStorage.getItem(previewKey) || '[]');
  let result: unknown;
  switch (name) {
    case 'list_assistants':
      result = assistants();
      break;
    case 'save_assistant': {
      const a = { ...(args.assistant as Assistant) };
      if (!a.name.trim() || !a.prompt.trim()) throw new Error('请填写助手名称与系统提示词');
      a.id ||= crypto.randomUUID();
      a.created_at ||= Date.now();
      a.updated_at = Date.now();
      localStorage.setItem(
        previewKey,
        JSON.stringify([a, ...assistants().filter((x) => x.id !== a.id)]),
      );
      result = a;
      break;
    }
    case 'delete_assistant':
      localStorage.setItem(
        previewKey,
        JSON.stringify(assistants().filter((x) => x.id !== args.id)),
      );
      result = null;
      break;
    case 'get_settings':
      result = defaultSettings;
      break;
    case 'list_devices':
    case 'list_sessions':
    case 'list_messages':
      result = [];
      break;
    case 'get_route':
      result =
        JSON.parse(localStorage.getItem(`vibesetter.preview.route.${args.id}`) || 'null') ||
        defaultRoute;
      break;
    case 'save_route':
      localStorage.setItem(`vibesetter.preview.route.${args.id}`, JSON.stringify(args.config));
      result = null;
      break;
    default:
      throw new Error('此功能需要在 Windows 或 macOS 桌面应用中使用；浏览器仅提供助手编辑预览。');
  }
  return result as T;
}

export async function onSessionEvent(callback: (event: SessionEvent) => void): Promise<() => void> {
  if (!isDesktop) return () => {};
  return listen<SessionEvent>('session-event', (event) => callback(event.payload));
}
