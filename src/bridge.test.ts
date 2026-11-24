import { beforeEach, describe, expect, it, vi } from 'vitest';

const memory = new Map<string, string>();
vi.stubGlobal('window', {});
vi.stubGlobal('localStorage', {
  getItem: (k: string) => memory.get(k) ?? null,
  setItem: (k: string, v: string) => memory.set(k, v),
});
const { command } = await import('./bridge');
import type { Assistant } from './types';

describe('browser preview boundary', () => {
  beforeEach(() => memory.clear());
  it('saves custom assistants with Unicode and updates without duplication', async () => {
    const a = await command<Assistant>('save_assistant', {
      assistant: {
        id: '',
        name: '研究搭档🎙️',
        prompt: '按我的提示词工作。',
        avatar: null,
        color: 'teal',
      },
    });
    await command('save_assistant', { assistant: { ...a, prompt: '新的职责' } });
    const saved = await command<Assistant[]>('list_assistants');
    expect(saved).toHaveLength(1);
    expect(saved[0].prompt).toBe('新的职责');
    expect(saved[0].name).toBe('研究搭档🎙️');
  });
  it('never stores credentials or pretends to connect to native audio', async () => {
    await expect(
      command('save_settings', { patch: { text_api_key: 'secret-test-value' } }),
    ).rejects.toThrow('桌面应用');
    await expect(command('start_session')).rejects.toThrow('桌面应用');
    expect(await command('list_devices')).toEqual([]);
    expect([...memory.values()].join('')).not.toContain('secret-test-value');
  });
});
