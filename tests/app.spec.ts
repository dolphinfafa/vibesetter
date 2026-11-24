import { expect, test } from '@playwright/test';

async function createAssistant(page: import('@playwright/test').Page) {
  await page.goto('/');
  await page.getByRole('button', { name: '创建你的助手', exact: true }).click();
  await page.getByLabel('助手名称').fill('我的思考搭档');
  await page
    .getByLabel('系统提示词')
    .fill('你是我的思考搭档，发现值得深入的问题，在适当的停顿简洁回应。');
  await page.getByRole('button', { name: '创建助手', exact: true }).click();
  await expect(page.getByRole('heading', { name: '我的思考搭档', exact: true })).toBeVisible();
}

test('custom assistant persists, editable prompt and multiple routes survive reload', async ({
  page,
}) => {
  await createAssistant(page);
  await page.getByRole('checkbox', { name: /电脑音频/ }).click();
  await expect(page.getByRole('checkbox', { name: /电脑音频/ })).toBeChecked();
  await expect(page.getByRole('checkbox', { name: /文字输入/ })).toBeChecked();
  await page.getByRole('button', { name: '编辑助手', exact: true }).click();
  await page.getByLabel('系统提示词').fill('新的角色由这段提示词定义，不受场景限制。');
  await page.getByRole('button', { name: '保存修改' }).click();
  await expect(page.getByText('新的角色由这段提示词定义，不受场景限制。')).toBeVisible();
  await page.reload();
  await page.getByRole('button', { name: '开启对话' }).click();
  await expect(page.getByRole('checkbox', { name: /电脑音频/ })).toBeChecked();
  await expect(page.getByText('新的角色由这段提示词定义，不受场景限制。')).toBeVisible();
  await page.screenshot({ path: '.artifacts/conversation.png', fullPage: true });
});

test('browser preview rejects native session and never fabricates messages', async ({ page }) => {
  await createAssistant(page);
  await page.getByRole('button', { name: '开始会话' }).click();
  await expect(page.getByRole('alert')).toContainText('桌面应用');
  await expect(page.getByLabel('文字输入', { exact: true })).toBeDisabled();
  await expect(page.locator('.message-row')).toHaveCount(0);
});

test('settings explain service setup and cannot persist a secret in browser storage', async ({
  page,
}) => {
  await page.goto('/');
  await page.getByRole('button', { name: '设置', exact: true }).click();
  await page.getByLabel('API Key', { exact: true }).fill('example-secret-for-test');
  await page.getByRole('button', { name: '保存设置' }).click();
  await expect(page.getByRole('alert')).toContainText('桌面应用');
  expect(await page.evaluate(() => JSON.stringify(localStorage))).not.toContain(
    'example-secret-for-test',
  );
});

test('delete removes an assistant after explicit confirmation', async ({ page }) => {
  await createAssistant(page);
  await page.getByRole('button', { name: '助手空间', exact: false }).first().click();
  await page.getByRole('button', { name: '删除我的思考搭档' }).click();
  await page.getByRole('button', { name: '删除助手', exact: true }).click();
  await expect(page.getByRole('heading', { name: '我的思考搭档', exact: true })).toHaveCount(0);
  await page.reload();
  await expect(page.getByText('还没有助手', { exact: false })).toBeVisible();
  await page.screenshot({ path: '.artifacts/home.png', fullPage: true });
});

test('desktop IPC supports multiple routes, locks active devices and preserves Chinese IME composition', async ({
  page,
}) => {
  await page.addInitScript(() => {
    const runtime = window as unknown as Record<string, unknown>;
    const assistant = {
      id: 'ipc-assistant',
      name: '🎙 搭档',
      prompt: '自定义角色',
      avatar: null,
      color: 'violet',
      created_at: 1,
      updated_at: 1,
    };
    let route = {
      microphone_ids: [] as string[],
      system_audio: false,
      text_input: true,
      text_output: true,
      output_ids: [] as string[],
    };
    runtime.__test_submitted = [];
    runtime.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: () => {} };
    runtime.__TAURI_INTERNALS__ = {
      transformCallback: () => 1,
      invoke: async (cmd: string, args: Record<string, unknown> = {}) => {
        switch (cmd) {
          case 'plugin:event|listen':
            return 1;
          case 'plugin:event|unlisten':
            return null;
          case 'list_assistants':
            return [assistant];
          case 'get_settings':
            return { has_text_api_key: true, text_model: 'test-model', has_speech_api_key: true };
          case 'list_devices':
            return [
              { id: 'input:test', name: '测试麦克风', kind: 'input', is_default: true },
              { id: 'output:headphones', name: '测试耳机', kind: 'output', is_default: true },
              { id: 'output:virtual', name: '测试虚拟声卡', kind: 'output', is_default: false },
            ];
          case 'get_route':
            return route;
          case 'save_route':
            route = args.config as typeof route;
            return null;
          case 'start_session':
            runtime.__test_start_config = args.config;
            return { id: 'ipc-session', assistant_id: assistant.id, started_at: 1, ended_at: null };
          case 'list_messages':
            return [];
          case 'submit_text':
            (runtime.__test_submitted as unknown[]).push(args.text);
            return null;
          default:
            throw new Error(`Unexpected test command: ${cmd}`);
        }
      },
    };
  });
  await page.goto('/');
  await page.getByRole('button', { name: '开启对话' }).click();
  await page.getByRole('checkbox', { name: /测试麦克风/ }).click();
  await page.getByRole('checkbox', { name: /电脑音频/ }).click();
  await page.getByRole('checkbox', { name: /测试耳机/ }).click();
  await page.getByRole('checkbox', { name: /测试虚拟声卡/ }).click();
  await page.getByRole('button', { name: '开始会话' }).click();
  await expect(page.getByRole('checkbox', { name: /测试麦克风/ })).toBeDisabled();
  expect(
    await page.evaluate(() => (window as unknown as Record<string, unknown>).__test_start_config),
  ).toEqual({
    microphone_ids: ['input:test'],
    system_audio: true,
    text_input: true,
    text_output: true,
    output_ids: ['output:headphones', 'output:virtual'],
  });
  const composer = page.getByLabel('文字输入', { exact: true });
  await composer.fill('中文尚在输入');
  await composer.dispatchEvent('keydown', {
    key: 'Enter',
    keyCode: 229,
    isComposing: true,
    bubbles: true,
  });
  await expect(composer).toHaveValue('中文尚在输入');
  expect(
    await page.evaluate(() => (window as unknown as Record<string, unknown>).__test_submitted),
  ).toEqual([]);
  await composer.press('Enter');
  await expect(composer).toHaveValue('');
  expect(
    await page.evaluate(() => (window as unknown as Record<string, unknown>).__test_submitted),
  ).toEqual(['中文尚在输入']);
});
