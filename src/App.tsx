import { useEffect, useRef, useState, type FormEvent, type ReactNode } from 'react';
import {
  Activity,
  ArrowDown,
  ArrowRight,
  AudioLines,
  Check,
  ChevronDown,
  ChevronRight,
  CircleHelp,
  Command,
  Headphones,
  History,
  LayoutGrid,
  LoaderCircle,
  MessageSquare,
  Mic,
  Monitor,
  Pause,
  Pencil,
  Play,
  Plus,
  Radio,
  RefreshCw,
  Send,
  Settings2,
  ShieldCheck,
  Sparkles,
  Square,
  Trash2,
  Upload,
  Volume2,
  WandSparkles,
  X,
  Zap,
} from 'lucide-react';
import { command, isDesktop, onSessionEvent } from './bridge';
import {
  defaultRoute,
  defaultSettings,
  type Assistant,
  type AudioDevice,
  type Message,
  type PublicSettings,
  type Session,
  type SessionConfig,
  type SessionEvent,
  type SettingsPatch,
} from './types';

type Page = 'home' | 'assistant' | 'settings';
const colors = ['violet', 'blue', 'teal', 'rose', 'amber'];
const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e));
const time = (timestamp: number) =>
  new Date(timestamp).toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit' });
const blankAssistant = (): Assistant => ({
  id: '',
  name: '',
  prompt: '',
  avatar: null,
  color: 'violet',
  created_at: 0,
  updated_at: 0,
});

function Avatar({ assistant, size = '' }: { assistant: Assistant; size?: string }) {
  return (
    <span className={`avatar ${assistant.color} ${size}`}>
      {assistant.avatar ? (
        <img src={assistant.avatar} alt="" />
      ) : (
        Array.from(assistant.name.trim())[0] || <Sparkles size={22} />
      )}
    </span>
  );
}
function IconButton({
  label,
  onClick,
  children,
  disabled = false,
  className = '',
}: {
  label: string;
  onClick: () => void;
  children: ReactNode;
  disabled?: boolean;
  className?: string;
}) {
  return (
    <button
      className={`icon-button ${className}`}
      aria-label={label}
      title={label}
      onClick={onClick}
      disabled={disabled}
    >
      {children}
    </button>
  );
}

export default function App() {
  const [page, setPage] = useState<Page>('home');
  const [assistants, setAssistants] = useState<Assistant[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [settings, setSettings] = useState<PublicSettings>(defaultSettings);
  const [devices, setDevices] = useState<AudioDevice[]>([]);
  const [route, setRoute] = useState<SessionConfig>(defaultRoute);
  const [editor, setEditor] = useState<Assistant | null>(null);
  const [session, setSession] = useState<Session | null>(null);
  const [status, setStatus] = useState('stopped');
  const [messages, setMessages] = useState<(Message & { visible?: boolean })[]>([]);
  const [partials, setPartials] = useState<Record<string, string>>({});
  const [levels, setLevels] = useState<Record<string, number>>({});
  const [text, setText] = useState('');
  const [busy, setBusy] = useState(false);
  const [toast, setToast] = useState<{ text: string; error: boolean } | null>(null);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [deleteHistory, setDeleteHistory] = useState<Session | null>(null);
  const [sessions, setSessions] = useState<Session[]>([]);
  const [viewingHistory, setViewingHistory] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState<Assistant | null>(null);
  const [help, setHelp] = useState(false);
  const currentSession = useRef<string | null>(null);
  const routeWrites = useRef<Promise<unknown>>(Promise.resolve());
  const bottom = useRef<HTMLDivElement>(null);
  const selected = assistants.find((a) => a.id === selectedId);
  const running = session !== null;
  const notify = (message: string, error = false) => setToast({ text: message, error });
  const fail = (e: unknown) => notify(errorText(e), true);

  useEffect(() => {
    Promise.all([
      command<Assistant[]>('list_assistants'),
      command<PublicSettings>('get_settings'),
      command<AudioDevice[]>('list_devices'),
    ])
      .then(([a, s, d]) => {
        setAssistants(a);
        setSettings(s);
        setDevices(d);
      })
      .catch(fail);
    let disposed = false;
    let unlisten: (() => void) | undefined;
    onSessionEvent((event: SessionEvent) => {
      const id = event.type === 'message' ? event.message.session_id : event.session_id;
      if (id !== currentSession.current) return;
      if (event.type === 'status') {
        setStatus(event.status);
        if (event.status === 'stopped') {
          currentSession.current = null;
          setSession(null);
          setPartials({});
          setLevels({});
        }
      } else if (event.type === 'message')
        setMessages((previous) =>
          previous.some((m) => m.id === event.message.id)
            ? previous
            : [...previous, { ...event.message, visible: event.visible }],
        );
      else if (event.type === 'transcript')
        setPartials((p) => ({ ...p, [event.source]: event.text }));
      else if (event.type === 'level') setLevels((p) => ({ ...p, [event.source]: event.value }));
      else if (event.type === 'interrupted')
        setMessages((p) =>
          p.map((m) => (m.id === event.message_id ? { ...m, interrupted: true } : m)),
        );
      else if (event.type === 'error') {
        const source: Record<string, string> = {
          system: '电脑音频',
          storage: '本地存储',
          text: '豆包模型',
          tts: '语音输出',
          assistant: '助手',
        };
        notify(`${source[event.source] || '音频设备'}：${event.message}`, true);
      }
    })
      .then((fn) => {
        if (disposed) fn();
        else unlisten = fn;
      })
      .catch(fail);
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);
  useEffect(() => {
    if (toast) {
      const timer = setTimeout(() => setToast(null), toast.error ? 10000 : 4000);
      return () => clearTimeout(timer);
    }
  }, [toast]);
  useEffect(() => {
    bottom.current?.scrollIntoView({ behavior: 'smooth', block: 'end' });
  }, [messages, partials, status]);
  useEffect(() => {
    if (!isDesktop) return;
    const timer = setInterval(() => {
      command<AudioDevice[]>('list_devices')
        .then(setDevices)
        .catch(() => {});
    }, 5000);
    return () => clearInterval(timer);
  }, []);

  async function stop() {
    await command('stop_session');
    currentSession.current = null;
    setSession(null);
    setStatus('stopped');
    setPartials({});
    setLevels({});
  }
  async function navigate(next: Page, assistant?: Assistant) {
    if (busy) return;
    try {
      await routeWrites.current.catch(() => undefined);
      if (session && (next !== 'assistant' || assistant?.id !== selectedId)) await stop();
      if (assistant) {
        const config = await command<SessionConfig>('get_route', { id: assistant.id });
        setSelectedId(assistant.id);
        setRoute(config);
        setMessages([]);
        setViewingHistory(false);
        setText('');
      }
      setPage(next);
      setHistoryOpen(false);
    } catch (e) {
      fail(e);
    }
  }
  async function start() {
    if (!selected) return;
    setBusy(true);
    try {
      await routeWrites.current;
      const s = await command<Session>('start_session', {
        assistantId: selected.id,
        config: route,
      });
      currentSession.current = s.id;
      setSession(s);
      setStatus('listening');
      setMessages([]);
      setPartials({});
      setViewingHistory(false);
      // Recover startup events that arrived before invoke resolved.
      const initial = await command<Message[]>('list_messages', { sessionId: s.id });
      setMessages((p) => [...initial.filter((m) => !p.some((x) => x.id === m.id)), ...p]);
    } catch (e) {
      fail(e);
    } finally {
      setBusy(false);
    }
  }
  async function updateRoute(config: SessionConfig) {
    setRoute(config);
    if (selected)
      try {
        const id = selected.id;
        routeWrites.current = routeWrites.current
          .catch(() => undefined)
          .then(() => command('save_route', { id, config }));
        await routeWrites.current;
      } catch (e) {
        fail(e);
      }
  }
  async function saveAssistant(assistant: Assistant) {
    const saved = await command<Assistant>('save_assistant', { assistant });
    setAssistants(await command<Assistant[]>('list_assistants'));
    setEditor(null);
    notify('助手已保存');
    if (!assistant.id) await navigate('assistant', saved);
  }
  async function deleteAssistant() {
    if (!confirmDelete) return;
    try {
      if (session?.assistant_id === confirmDelete.id) await stop();
      await command('delete_assistant', { id: confirmDelete.id });
      setAssistants((a) => a.filter((x) => x.id !== confirmDelete.id));
      if (selectedId === confirmDelete.id) {
        setPage('home');
        setSelectedId(null);
      }
      setConfirmDelete(null);
      notify('助手及其会话记录已删除');
    } catch (e) {
      fail(e);
    }
  }
  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!text.trim() || !session) return;
    const value = text;
    try {
      await command('submit_text', { text: value });
      setText('');
    } catch (e) {
      fail(e);
    }
  }
  async function openHistory() {
    if (!selected) return;
    try {
      setSessions(await command<Session[]>('list_sessions', { assistantId: selected.id }));
      setHistoryOpen(true);
    } catch (e) {
      fail(e);
    }
  }
  async function loadHistory(s: Session) {
    try {
      if (running) await stop();
      setMessages(await command<Message[]>('list_messages', { sessionId: s.id }));
      setViewingHistory(true);
      setHistoryOpen(false);
    } catch (e) {
      fail(e);
    }
  }
  const deviceName = (id: string) =>
    id === 'system' ? '电脑音频' : devices.find((d) => d.id === id)?.name || id;
  const activeStatus =
    (
      {
        listening: '正在倾听',
        thinking: '正在思考',
        speaking: '正在回应',
        stopped: '未开始',
      } as Record<string, string>
    )[status] || status;

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <button className="brand" onClick={() => void navigate('home')}>
          <span className="brand-mark">
            <AudioLines size={24} strokeWidth={2.5} />
          </span>
          <span>
            vibesetter<span className="brand-dot">.</span>
          </span>
        </button>
        <div className="workspace-label">
          你的个人工作空间 <ChevronDown size={13} />
        </div>
        <nav className="primary-nav">
          <button className={page === 'home' ? 'active' : ''} onClick={() => void navigate('home')}>
            <LayoutGrid size={18} />
            助手空间<span className="nav-count">{assistants.length}</span>
          </button>
        </nav>
        <div className="sidebar-section-title">
          <span>我的助手</span>
          <IconButton label="新建助手" onClick={() => setEditor(blankAssistant())}>
            <Plus size={16} />
          </IconButton>
        </div>
        <div className="assistant-nav">
          {assistants.map((a) => (
            <button
              key={a.id}
              className={page === 'assistant' && selectedId === a.id ? 'active' : ''}
              onClick={() => void navigate('assistant', a)}
            >
              <Avatar assistant={a} size="tiny" />
              <span>{a.name}</span>
              {session?.assistant_id === a.id && <span className="live-dot" />}
            </button>
          ))}
          {assistants.length === 0 && (
            <div className="sidebar-empty">
              还没有助手
              <br />
              <span>创建一位，开始你的灵感之旅</span>
            </div>
          )}
        </div>
        <div className="sidebar-bottom">
          <div className="sidebar-tip">
            <span className="tip-icon">
              <Sparkles size={17} />
            </span>
            <strong>好的想法，值得被接住</strong>
            <p>
              让助手倾听、思考，
              <br />
              陪你把想法向前推进一步。
            </p>
          </div>
          <button
            className={page === 'settings' ? 'bottom-link active' : 'bottom-link'}
            onClick={() => void navigate('settings')}
          >
            <Settings2 size={18} />
            设置{!settings.has_text_api_key && <span className="setup-dot" />}
          </button>
          <button className="bottom-link" onClick={() => setHelp(true)}>
            <CircleHelp size={18} />
            使用指南
            <ArrowRight size={14} />
          </button>
          <div className="profile">
            <span className="profile-avatar">我</span>
            <div>
              <strong>我的工作空间</strong>
              <small>本地存储 · 自由创造</small>
            </div>
            <span className="version">v0.1</span>
          </div>
        </div>
      </aside>

      <main className={`main-area ${page === 'assistant' ? 'conversation-layout' : ''}`}>
        <header className="topbar">
          <div className="breadcrumb">
            <span>工作台</span>
            <ChevronRight size={14} />
            <strong>
              {page === 'home'
                ? '助手空间'
                : page === 'settings'
                  ? '设置'
                  : selected?.name || '助手'}
            </strong>
          </div>
          <div className="topbar-right">
            {!isDesktop && <span className="preview-badge">浏览器预览</span>}
            <span className={`connection-badge ${settings.has_text_api_key ? 'configured' : ''}`}>
              <span />
              {settings.has_text_api_key ? '豆包已配置' : '豆包未配置'}
            </span>
            <span className="topbar-divider" />
            <IconButton label="使用指南" onClick={() => setHelp(true)}>
              <CircleHelp size={18} />
            </IconButton>
          </div>
        </header>

        {page === 'home' && (
          <div className="dashboard">
            <section className="hero">
              <div className="hero-copy">
                <div className="eyebrow">
                  <span /> YOUR IDEAS, IN GOOD COMPANY
                </div>
                <h1>
                  给每个灵感，
                  <br />
                  一个默契的<span>搭档。</span>
                </h1>
                <p>
                  定义你的助手，连接你的声音。
                  <br />
                  从倾听到回应，让每一段对话都有新的可能。
                </p>
                <button className="button primary" onClick={() => setEditor(blankAssistant())}>
                  <Plus size={17} />
                  创建你的助手
                  <ArrowRight size={16} />
                </button>
                <div className="hero-features">
                  <span>
                    <AudioLines size={14} />
                    实时倾听
                  </span>
                  <span>
                    <WandSparkles size={14} />
                    由你定义
                  </span>
                  <span>
                    <ShieldCheck size={14} />
                    本地存储
                  </span>
                </div>
              </div>
              <div className="hero-art" aria-hidden="true">
                <div className="orbit orbit-one" />
                <div className="orbit orbit-two" />
                <div className="orbit orbit-three" />
                <div className="orb">
                  <AudioLines size={66} strokeWidth={1.4} />
                </div>
                <div className="floating-tag tag-a">
                  <Mic size={16} />
                  <span>倾听你的声音</span>
                  <div className="mini-wave">
                    <i />
                    <i />
                    <i />
                    <i />
                    <i />
                  </div>
                </div>
                <div className="floating-tag tag-b">
                  <Sparkles size={17} />
                  <span>让灵感自然发生</span>
                </div>
                <span className="art-spark spark-a">✧</span>
                <span className="art-spark spark-b">✦</span>
                <span className="art-dot dot-a" />
                <span className="art-dot dot-b" />
              </div>
            </section>
            {!settings.has_text_api_key && (
              <button className="setup-banner" onClick={() => void navigate('settings')}>
                <span className="setup-banner-icon">
                  <Zap size={19} />
                </span>
                <div>
                  <strong>连接豆包，开启你的第一段对话</strong>
                  <span>配置 API Key 后，助手就可以开始思考和回应。</span>
                </div>
                <span className="setup-link">
                  前往设置
                  <ArrowRight size={16} />
                </span>
              </button>
            )}
            <section className="assistants-section">
              <div className="section-heading">
                <div>
                  <h2>
                    我的助手 <span>{assistants.length.toString().padStart(2, '0')}</span>
                  </h2>
                  <p>各有专长，都由你来定义。</p>
                </div>
                <button
                  className="button secondary small"
                  onClick={() => setEditor(blankAssistant())}
                >
                  <Plus size={16} />
                  新建助手
                </button>
              </div>
              <div className="assistant-grid">
                {assistants.map((a) => (
                  <article className="assistant-card" key={a.id}>
                    <div className="card-top">
                      <Avatar assistant={a} />
                      <div className="card-actions">
                        <IconButton label={`编辑${a.name}`} onClick={() => setEditor(a)}>
                          <Pencil size={15} />
                        </IconButton>
                        <IconButton label={`删除${a.name}`} onClick={() => setConfirmDelete(a)}>
                          <Trash2 size={15} />
                        </IconButton>
                      </div>
                    </div>
                    <h3>{a.name}</h3>
                    <p>{a.prompt}</p>
                    <button className="card-open" onClick={() => void navigate('assistant', a)}>
                      开启对话
                      <ArrowRight size={16} />
                    </button>
                  </article>
                ))}
                <button
                  className={`create-card ${assistants.length === 0 ? 'first-create' : ''}`}
                  onClick={() => setEditor(blankAssistant())}
                >
                  <span className="create-plus">
                    <Plus size={24} />
                  </span>
                  <strong>{assistants.length === 0 ? '你的第一位助手' : '一个新的灵感搭档'}</strong>
                  <span>
                    一个名字，一段提示词
                    <br />
                    把你的想法变成专属助手
                  </span>
                  <span className="create-link">
                    开始创建
                    <ArrowRight size={14} />
                  </span>
                </button>
              </div>
            </section>
            <section className="how-it-works">
              <div className="how-heading">
                <span className="overline">MADE FOR YOUR FLOW</span>
                <h2>顺着你的节奏，融入你的工作</h2>
              </div>
              <div className="steps">
                <div>
                  <span className="step-number">01</span>
                  <span className="step-icon">
                    <WandSparkles size={20} />
                  </span>
                  <h3>定义一个角色</h3>
                  <p>
                    用提示词告诉助手，它是谁，
                    <br />
                    以及你希望它如何帮助你。
                  </p>
                </div>
                <div>
                  <span className="step-number">02</span>
                  <span className="step-icon">
                    <Radio size={20} />
                  </span>
                  <h3>连接输入与输出</h3>
                  <p>
                    组合麦克风、电脑音频和文字，
                    <br />
                    选择你希望接收回应的方式。
                  </p>
                </div>
                <div>
                  <span className="step-number">03</span>
                  <span className="step-icon">
                    <MessageSquare size={20} />
                  </span>
                  <h3>让对话自然发生</h3>
                  <p>
                    实时建议，或开口回应，
                    <br />
                    你的助手会跟上你的节奏。
                  </p>
                </div>
              </div>
            </section>
            <footer className="dashboard-footer">
              <span>
                <ShieldCheck size={13} />
                助手与文字记录保存在本机 · 启用的输入会发送至所配置的模型服务
              </span>
              <span>Made for your next good idea.</span>
            </footer>
          </div>
        )}

        {page === 'assistant' && selected && (
          <div className="assistant-workspace">
            <section className="conversation">
              <div className="conversation-header">
                <div className="conversation-identity">
                  <Avatar assistant={selected} size="medium" />
                  <div>
                    <h1>{selected.name}</h1>
                    <span className={`session-status ${running ? 'live' : ''}`}>
                      <span />
                      {viewingHistory ? '历史会话' : activeStatus}
                    </span>
                  </div>
                </div>
                <div className="conversation-actions">
                  <IconButton label="编辑助手" onClick={() => setEditor(selected)}>
                    <Pencil size={17} />
                  </IconButton>
                  <IconButton label="会话历史" onClick={() => void openHistory()}>
                    <History size={18} />
                  </IconButton>
                  <button
                    className={`button ${running ? 'secondary' : 'primary'} small`}
                    disabled={busy}
                    onClick={() => {
                      if (running) stop().catch(fail);
                      else void start();
                    }}
                  >
                    {busy ? (
                      <LoaderCircle size={15} className="spin" />
                    ) : running ? (
                      <Pause size={15} />
                    ) : (
                      <Play size={15} />
                    )}
                    {running ? '暂停会话' : '开始会话'}
                  </button>
                </div>
              </div>
              <div className="message-list">
                {messages.length === 0 && (
                  <div className="conversation-empty">
                    <div className="empty-orb">
                      <AudioLines size={34} />
                    </div>
                    <h2>{running ? '准备好了，我在听' : `和${selected.name}一起开始`}</h2>
                    <p>
                      {running
                        ? '说说你的想法，或在下方输入文字。'
                        : '在右侧选择输入与输出，开始一段新的对话。'}
                    </p>
                    <div className="prompt-preview">
                      <span>
                        <Command size={14} />
                        系统提示词
                      </span>
                      <p>{selected.prompt}</p>
                    </div>
                    {!isDesktop && (
                      <div className="native-note">实时音频和模型连接需要在桌面应用中使用。</div>
                    )}
                  </div>
                )}
                {messages.map((m) => (
                  <div
                    key={m.id}
                    className={`message-row ${m.role === 'assistant' ? 'assistant-message' : 'user-message'}`}
                  >
                    {m.role === 'assistant' ? (
                      <Avatar assistant={selected} size="tiny" />
                    ) : (
                      <span className="message-source-icon">
                        {m.source === '文字输入' ? <MessageSquare size={16} /> : <Mic size={16} />}
                      </span>
                    )}
                    <div className="message-content">
                      <div className="message-meta">
                        <strong>
                          {m.role === 'assistant' ? selected.name : deviceName(m.source)}
                        </strong>
                        <time>{time(m.created_at)}</time>
                        {m.interrupted && <small>已打断</small>}
                      </div>
                      <div className="message-bubble">
                        {m.visible === false ? (
                          <span className="audio-reply">
                            <Volume2 size={16} />
                            语音回应
                          </span>
                        ) : (
                          m.text
                        )}
                      </div>
                    </div>
                  </div>
                ))}
                {Object.entries(partials)
                  .filter(([, t]) => t)
                  .map(([source, value]) => (
                    <div className="partial-transcript" key={source}>
                      <AudioLines size={15} />
                      <span>
                        {deviceName(source)}：{value}
                      </span>
                    </div>
                  ))}
                {status === 'thinking' && (
                  <div className="thinking-indicator">
                    <span />
                    <span />
                    <span />
                    <small>正在整理思路</small>
                  </div>
                )}
                <div ref={bottom} />
              </div>
              <div className="composer-area">
                {running && (
                  <div className="live-controls">
                    <span>
                      <span className="live-dot" />
                      {activeStatus}
                    </span>
                    <div>
                      <button onClick={() => command('trigger_reply').catch(fail)}>
                        <Sparkles size={13} />
                        回应一次
                      </button>
                      <button onClick={() => command('interrupt_reply').catch(fail)}>
                        <Square size={12} />
                        立即停止
                      </button>
                    </div>
                  </div>
                )}
                <form className="composer" onSubmit={submit}>
                  <textarea
                    aria-label="文字输入"
                    placeholder={
                      !route.text_input
                        ? '在右侧启用文字输入'
                        : !running
                          ? '开始会话后，在这里输入你的想法…'
                          : '输入你的想法，Enter 发送，Shift + Enter 换行…'
                    }
                    value={text}
                    disabled={!running || !route.text_input}
                    maxLength={12000}
                    onChange={(e) => setText(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.nativeEvent.isComposing || e.keyCode === 229) return;
                      if (e.key === 'Enter' && !e.shiftKey) {
                        e.preventDefault();
                        if (text.trim()) void submit(e);
                      }
                    }}
                  />
                  <div className="composer-bottom">
                    <span>
                      <ShieldCheck size={12} />
                      不保存原始音频
                    </span>
                    <button
                      aria-label="发送文字"
                      className="send-button"
                      disabled={!running || !route.text_input || !text.trim()}
                      type="submit"
                    >
                      <Send size={16} />
                    </button>
                  </div>
                </form>
                <p className="composer-disclaimer">AI 的建议供你参考，重要判断由你决定。</p>
              </div>
            </section>
            <aside className="routing-panel">
              <div className="routing-title">
                <h2>连接与路由</h2>
                <IconButton
                  label="刷新音频设备"
                  onClick={() =>
                    command<AudioDevice[]>('list_devices')
                      .then(setDevices)
                      .then(() => notify('设备列表已刷新'))
                      .catch(fail)
                  }
                >
                  <RefreshCw size={15} />
                </IconButton>
              </div>
              <p className="routing-intro">自由组合，让助手融入你的工作流。</p>
              <RouteGroup title="输入" subtitle="助手可以接收什么" icon={<ArrowDown size={15} />}>
                <DeviceToggle
                  label="文字输入"
                  description="手动输入想法与指令"
                  icon={<MessageSquare size={17} />}
                  checked={route.text_input}
                  disabled={running || busy}
                  onChange={() => void updateRoute({ ...route, text_input: !route.text_input })}
                />
                <DeviceToggle
                  label="电脑音频"
                  description="采集系统声音，排除自身播放"
                  icon={<Monitor size={17} />}
                  checked={route.system_audio}
                  disabled={running || busy}
                  level={levels.system}
                  onChange={() => void updateRoute({ ...route, system_audio: !route.system_audio })}
                />
                {devices
                  .filter((d) => d.kind === 'input')
                  .map((d) => (
                    <DeviceToggle
                      key={d.id}
                      label={d.name}
                      description={d.is_default ? '默认麦克风' : '麦克风'}
                      icon={<Mic size={17} />}
                      checked={route.microphone_ids.includes(d.id)}
                      disabled={running || busy}
                      level={levels[d.id]}
                      onChange={() =>
                        void updateRoute({
                          ...route,
                          microphone_ids: toggle(route.microphone_ids, d.id),
                        })
                      }
                    />
                  ))}
                {route.microphone_ids
                  .filter((id) => !devices.some((d) => d.id === id))
                  .map((id) => (
                    <DeviceToggle
                      key={id}
                      label="麦克风已断开"
                      description="取消选择后可重新配置"
                      icon={<Mic size={17} />}
                      checked
                      disabled={running || busy}
                      onChange={() =>
                        void updateRoute({
                          ...route,
                          microphone_ids: toggle(route.microphone_ids, id),
                        })
                      }
                    />
                  ))}
                {!devices.some((d) => d.kind === 'input') && (
                  <p className="device-note">
                    {isDesktop
                      ? '未发现麦克风，请连接设备后刷新。'
                      : '桌面应用中将显示可用麦克风。'}
                  </p>
                )}
              </RouteGroup>
              <div className="routing-flow">
                <span />
                <AudioLines size={16} />
                <span />
              </div>
              <RouteGroup title="输出" subtitle="你希望如何收到回应" icon={<ArrowDown size={15} />}>
                <DeviceToggle
                  label="文字回应"
                  description="在对话中显示助手的建议"
                  icon={<MessageSquare size={17} />}
                  checked={route.text_output}
                  disabled={running || busy}
                  onChange={() => void updateRoute({ ...route, text_output: !route.text_output })}
                />
                {devices
                  .filter((d) => d.kind === 'output')
                  .map((d) => (
                    <DeviceToggle
                      key={d.id}
                      label={d.name}
                      description={d.is_default ? '默认音频输出' : '音频输出'}
                      icon={<Headphones size={17} />}
                      checked={route.output_ids.includes(d.id)}
                      disabled={running || busy}
                      onChange={() =>
                        void updateRoute({ ...route, output_ids: toggle(route.output_ids, d.id) })
                      }
                    />
                  ))}
                {route.output_ids
                  .filter((id) => !devices.some((d) => d.id === id))
                  .map((id) => (
                    <DeviceToggle
                      key={id}
                      label="输出设备已断开"
                      description="取消选择后可重新配置"
                      icon={<Headphones size={17} />}
                      checked
                      disabled={running || busy}
                      onChange={() =>
                        void updateRoute({ ...route, output_ids: toggle(route.output_ids, id) })
                      }
                    />
                  ))}
                {!devices.some((d) => d.kind === 'output') && (
                  <p className="device-note">
                    {isDesktop ? '未发现音频输出设备。' : '桌面应用中将显示耳机与虚拟声卡。'}
                  </p>
                )}
              </RouteGroup>
              {running && (
                <p className="route-lock">
                  <Pause size={12} />
                  暂停会话后可调整路由
                </p>
              )}
              <div className="route-tip">
                <Headphones size={18} />
                <strong>戴上耳机，聊得更自然</strong>
                <p>使用虚拟声卡可将助手声音送入播客软件；请避免将其回送到输入。</p>
                <button onClick={() => setHelp(true)}>
                  了解音频连接
                  <ArrowRight size={13} />
                </button>
              </div>
            </aside>
          </div>
        )}

        {page === 'settings' && (
          <SettingsPage settings={settings} onSaved={setSettings} notify={notify} />
        )}
      </main>
      {editor && (
        <AssistantEditor initial={editor} onClose={() => setEditor(null)} onSave={saveAssistant} />
      )}
      {confirmDelete && (
        <Modal title="删除助手？" onClose={() => setConfirmDelete(null)}>
          <p className="modal-copy">
            将删除「{confirmDelete.name}」和它的全部本地会话记录，此操作无法撤销。
          </p>
          <div className="modal-actions">
            <button className="button secondary" onClick={() => setConfirmDelete(null)}>
              取消
            </button>
            <button className="button danger" onClick={() => void deleteAssistant()}>
              删除助手
            </button>
          </div>
        </Modal>
      )}
      {historyOpen && (
        <Modal title="会话历史" onClose={() => setHistoryOpen(false)}>
          {sessions.length === 0 ? (
            <p className="modal-copy">还没有会话记录。开始对话后，文字记录会保存在本机。</p>
          ) : (
            <div className="history-list">
              {sessions.map((s) => (
                <div key={s.id}>
                  <button onClick={() => void loadHistory(s)}>
                    <History size={17} />
                    <span>
                      {new Date(s.started_at).toLocaleString('zh-CN')}
                      <small>{s.ended_at ? '已结束' : '进行中'}</small>
                    </span>
                    <ChevronRight size={15} />
                  </button>
                  <IconButton
                    label="删除此会话"
                    disabled={s.id === session?.id}
                    onClick={() => {
                      setDeleteHistory(s);
                    }}
                  >
                    <Trash2 size={15} />
                  </IconButton>
                </div>
              ))}
            </div>
          )}
        </Modal>
      )}
      {deleteHistory && (
        <Modal title="删除这段会话？" onClose={() => setDeleteHistory(null)}>
          <p className="modal-copy">这段会话的全部本地文字记录将被删除，无法撤销。</p>
          <div className="modal-actions">
            <button className="button secondary" onClick={() => setDeleteHistory(null)}>
              取消
            </button>
            <button
              className="button danger"
              onClick={() => {
                const id = deleteHistory.id;
                command('delete_session', { sessionId: id })
                  .then(() => {
                    setSessions((p) => p.filter((x) => x.id !== id));
                    setDeleteHistory(null);
                    notify('会话记录已删除');
                  })
                  .catch(fail);
              }}
            >
              删除会话
            </button>
          </div>
        </Modal>
      )}
      {help && (
        <Modal title="让你的助手开始工作" onClose={() => setHelp(false)}>
          <div className="help-content">
            <h3>1. 定义助手</h3>
            <p>填写名称和系统提示词。角色、交流风格、何时发言都可以写在提示词里。</p>
            <h3>2. 连接豆包与语音服务</h3>
            <p>
              在设置页配置文本模型。使用音频时，还需要开通火山语音识别和合成服务；可使用语音 API
              Key，或旧版 App ID 与 Token。连接检测会产生少量 API 用量。
            </p>
            <h3>3. 组合输入与输出</h3>
            <p>
              每类均可多选。推荐佩戴耳机；Mac
              首次采集需要授予麦克风和系统音频录制权限，修改权限后可能需要重启应用。
            </p>
            <h3>把声音送进播客软件</h3>
            <p>
              安装虚拟声卡（例如 macOS 的 BlackHole 或 Windows 的
              VB-CABLE），在本应用中选择它的播放端，在播客软件中选择对应的录音端。可同时勾选耳机监听。不要把该虚拟声卡再回送到本应用的输入。
            </p>
            <h3>数据与控制</h3>
            <p>
              文字记录和凭据保存在本机；启用的语音、文字和提示词会发送到所配置的服务。原始音频不落盘。暂停会话停止采集；立即停止取消当前回应，之后继续倾听。
            </p>
          </div>
        </Modal>
      )}
      {toast && (
        <div
          className={`toast ${toast.error ? 'error' : ''}`}
          role={toast.error ? 'alert' : 'status'}
        >
          {toast.error ? <CircleHelp size={18} /> : <Check size={18} />}
          <span>{toast.text}</span>
          <IconButton label="关闭提示" onClick={() => setToast(null)}>
            <X size={15} />
          </IconButton>
        </div>
      )}
    </div>
  );
}

function toggle(items: string[], id: string) {
  return items.includes(id) ? items.filter((x) => x !== id) : [...items, id];
}
function RouteGroup({
  title,
  subtitle,
  icon,
  children,
}: {
  title: string;
  subtitle: string;
  icon: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="route-group">
      <div className="route-group-title">
        <span>
          {icon}
          {title}
        </span>
        <small>可多选</small>
      </div>
      <p>{subtitle}</p>
      <div className="route-devices">{children}</div>
    </section>
  );
}
function DeviceToggle({
  label,
  description,
  icon,
  checked,
  disabled,
  onChange,
  level,
}: {
  label: string;
  description: string;
  icon: ReactNode;
  checked: boolean;
  disabled: boolean;
  onChange: () => void;
  level?: number;
}) {
  return (
    <button
      role="checkbox"
      aria-checked={checked}
      className={`device-toggle ${checked ? 'selected' : ''}`}
      onClick={onChange}
      disabled={disabled}
    >
      <span className="device-icon">{icon}</span>
      <span className="device-copy">
        <strong>{label}</strong>
        <small>{description}</small>
        {level !== undefined && checked && (
          <span className="level-meter">
            <i style={{ width: `${level * 100}%` }} />
          </span>
        )}
      </span>
      <span className="checkbox">{checked && <Check size={11} strokeWidth={3} />}</span>
    </button>
  );
}
function Modal({
  title,
  children,
  onClose,
  wide = false,
}: {
  title: string;
  children: ReactNode;
  onClose: () => void;
  wide?: boolean;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const node = ref.current;
    node?.showModal();
    return () => node?.close();
  }, []);
  return (
    <dialog
      className={`modal ${wide ? 'wide' : ''}`}
      ref={ref}
      onCancel={(e) => {
        e.preventDefault();
        onClose();
      }}
      onClick={(e) => {
        if (e.target === e.currentTarget) {
          const r = e.currentTarget.getBoundingClientRect();
          if (
            e.clientX < r.left ||
            e.clientX > r.right ||
            e.clientY < r.top ||
            e.clientY > r.bottom
          )
            onClose();
        }
      }}
    >
      <div className="modal-heading">
        <h2>{title}</h2>
        <IconButton label="关闭弹窗" onClick={onClose}>
          <X size={19} />
        </IconButton>
      </div>
      {children}
    </dialog>
  );
}

function AssistantEditor({
  initial,
  onClose,
  onSave,
}: {
  initial: Assistant;
  onClose: () => void;
  onSave: (a: Assistant) => Promise<void>;
}) {
  const [draft, setDraft] = useState(initial);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const upload = useRef<HTMLInputElement>(null);
  async function avatar(file?: File) {
    if (!file) return;
    if (!['image/png', 'image/jpeg', 'image/webp'].includes(file.type) || file.size > 10_000_000) {
      setError('请选择小于 10 MB 的 PNG、JPEG 或 WebP 图片');
      return;
    }
    try {
      const image = await createImageBitmap(file);
      const canvas = document.createElement('canvas');
      canvas.width = 256;
      canvas.height = 256;
      const ctx = canvas.getContext('2d');
      if (!ctx) throw new Error('无法处理图片');
      const size = Math.min(image.width, image.height);
      ctx.drawImage(
        image,
        (image.width - size) / 2,
        (image.height - size) / 2,
        size,
        size,
        0,
        0,
        256,
        256,
      );
      image.close();
      setDraft((p) => ({ ...p, avatar: canvas.toDataURL('image/png') }));
      setError('');
    } catch {
      setError('无法读取这张图片，请更换文件');
    }
  }
  return (
    <Modal title={initial.id ? '编辑助手' : '创建你的助手'} onClose={onClose} wide>
      <p className="modal-subtitle">从一个角色开始，创造与你默契的搭档。</p>
      <form
        onSubmit={async (e) => {
          e.preventDefault();
          setBusy(true);
          setError('');
          try {
            await onSave(draft);
          } catch (e) {
            setError(errorText(e));
          } finally {
            setBusy(false);
          }
        }}
      >
        <div className="avatar-editor">
          <Avatar assistant={draft} size="large" />
          <div>
            <button
              type="button"
              className="button secondary small"
              onClick={() => upload.current?.click()}
            >
              <Upload size={14} />
              上传头像
            </button>
            <small>PNG、JPEG 或 WebP，自动裁切为方形</small>
          </div>
          <input
            ref={upload}
            type="file"
            accept="image/png,image/jpeg,image/webp"
            hidden
            onChange={(e) => void avatar(e.target.files?.[0])}
          />
        </div>
        <div className="color-picker">
          {colors.map((color) => (
            <button
              type="button"
              aria-label={`${color}头像颜色`}
              key={color}
              className={`color-swatch ${color} ${draft.color === color ? 'chosen' : ''}`}
              onClick={() => setDraft((p) => ({ ...p, color }))}
            >
              {draft.color === color && <Check size={12} />}
            </button>
          ))}
          {draft.avatar && (
            <button
              type="button"
              className="text-button"
              onClick={() => setDraft((p) => ({ ...p, avatar: null }))}
            >
              移除图片
            </button>
          )}
        </div>
        <label className="field-label" htmlFor="assistant-name">
          助手名称 <span>*</span>
        </label>
        <input
          id="assistant-name"
          autoFocus
          required
          maxLength={80}
          placeholder="给你的搭档起个名字"
          value={draft.name}
          onChange={(e) => setDraft((p) => ({ ...p, name: e.target.value }))}
        />
        <div className="field-label-row">
          <label className="field-label" htmlFor="assistant-prompt">
            系统提示词 <span>*</span>
          </label>
          <span>{draft.prompt.length.toLocaleString()} / 32,000</span>
        </div>
        <textarea
          id="assistant-prompt"
          required
          maxLength={32000}
          rows={8}
          placeholder={
            '描述助手的角色、目标和交流方式。\n\n例如：你是我的思考搭档。倾听对话，发现值得深入的观点，在合适的停顿提出一个有启发性的问题。保持自然、简洁，避免频繁打断。'
          }
          value={draft.prompt}
          onChange={(e) => setDraft((p) => ({ ...p, prompt: e.target.value }))}
        />
        <div className="field-hint">
          <Sparkles size={13} />
          越具体的描述，越贴近你的期待。也可以写明何时回应、何时倾听。
        </div>
        {error && (
          <p className="inline-error" role="alert">
            {error}
          </p>
        )}
        <div className="modal-actions">
          <button type="button" className="button secondary" onClick={onClose}>
            取消
          </button>
          <button type="submit" className="button primary" disabled={busy}>
            {busy ? <LoaderCircle size={15} className="spin" /> : <Check size={15} />}
            {initial.id ? '保存修改' : '创建助手'}
          </button>
        </div>
      </form>
    </Modal>
  );
}

function SettingsPage({
  settings,
  onSaved,
  notify,
}: {
  settings: PublicSettings;
  onSaved: (s: PublicSettings) => void;
  notify: (text: string, error?: boolean) => void;
}) {
  const [draft, setDraft] = useState<SettingsPatch>({ ...settings });
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState('');
  const [advanced, setAdvanced] = useState(false);
  const update = (key: keyof SettingsPatch, value: string) =>
    setDraft((p) => ({ ...p, [key]: value }));
  async function persist() {
    const saved = await command<PublicSettings>('save_settings', { patch: draft });
    onSaved(saved);
    setDraft({ ...saved });
    return saved;
  }
  async function test(service: string) {
    setTesting(service);
    try {
      await persist();
      const result = await command<string>('test_connection', { service });
      notify(result);
    } catch (e) {
      notify(errorText(e), true);
    } finally {
      setTesting('');
    }
  }
  const testButton = (service: string, label: string) => (
    <button
      className="button secondary small"
      type="button"
      disabled={!!testing || saving}
      onClick={() => void test(service)}
    >
      {testing === service ? <LoaderCircle size={14} className="spin" /> : <Activity size={14} />}
      {label}
    </button>
  );
  const field = (
    key: keyof SettingsPatch,
    label: string,
    placeholder = '',
    secret = false,
    configured = false,
  ) => (
    <div className="settings-field" key={key}>
      <label className="field-label" htmlFor={key}>
        {label}
        {configured && (
          <span className="saved-label">
            <Check size={11} />
            已保存
          </span>
        )}
      </label>
      <input
        id={key}
        type={secret ? 'password' : 'text'}
        autoComplete="off"
        spellCheck={false}
        value={draft[key] || ''}
        placeholder={configured ? '已保存，留空保持原值' : placeholder}
        onChange={(e) => update(key, e.target.value)}
      />
      {secret && configured && (
        <button type="button" className="clear-secret" onClick={() => update(key, '')}>
          清除已保存凭据
        </button>
      )}
    </div>
  );
  return (
    <div className="settings-page">
      <div className="page-heading">
        <span className="overline">MAKE IT YOURS</span>
        <h1>设置</h1>
        <p>连接模型与声音，为你的助手注入能力。</p>
      </div>
      {!isDesktop && (
        <div className="preview-notice">
          <Monitor size={18} />
          <span>当前为浏览器预览。凭据配置和连接检测需要在桌面应用中完成。</span>
        </div>
      )}
      <form
        onSubmit={async (e) => {
          e.preventDefault();
          setSaving(true);
          try {
            await persist();
            notify('设置已保存，下次开始会话时生效');
          } catch (e) {
            notify(errorText(e), true);
          } finally {
            setSaving(false);
          }
        }}
      >
        <section className="settings-card">
          <div className="settings-card-heading">
            <span className="settings-service-icon violet">
              <Sparkles size={21} />
            </span>
            <div>
              <h2>豆包文本模型</h2>
              <p>理解对话、生成建议与回应</p>
            </div>
            {testButton('text', '检测连接')}
          </div>
          {field(
            'text_api_key',
            'API Key',
            '输入火山方舟 API Key',
            true,
            settings.has_text_api_key,
          )}
          {field(
            'text_model',
            '模型标识 / Endpoint ID',
            '输入支持 JSON 输出的模型 ID 或推理接入点 ID',
          )}
          <p className="settings-note">在火山方舟控制台创建 API Key，并填入已开通模型的标识。</p>
        </section>
        <section className="settings-card">
          <div className="settings-card-heading">
            <span className="settings-service-icon teal">
              <AudioLines size={21} />
            </span>
            <div>
              <h2>火山语音服务</h2>
              <p>将声音变成文字，让助手开口交流</p>
            </div>
          </div>
          {field(
            'speech_api_key',
            '语音 API Key · 新版控制台',
            '输入语音服务 API Key',
            true,
            settings.has_speech_api_key,
          )}
          <div className="or-divider">
            <span />
            或使用旧版凭据
            <span />
          </div>
          <div className="two-fields">
            {field('speech_app_id', 'App ID', '语音服务 App ID')}
            {field(
              'speech_access_token',
              'Access Token',
              '语音服务 Access Token',
              true,
              settings.has_speech_access_token,
            )}
          </div>
          <p className="settings-note">
            新版 Key 与旧版凭据填写一组即可；同时配置时优先使用新版
            Key。请分别开通语音识别与合成资源。
          </p>
          <div className="settings-tests">
            {testButton('asr', '检测语音识别')}
            {testButton('tts', '检测语音合成')}
            <small>检测会产生少量 API 用量</small>
          </div>
        </section>
        <section className="settings-card advanced-card">
          <button className="advanced-toggle" type="button" onClick={() => setAdvanced(!advanced)}>
            <Settings2 size={18} />
            <span>
              高级服务配置<small>资源标识、音色与接口地址</small>
            </span>
            <ChevronDown size={17} className={advanced ? 'rotated' : ''} />
          </button>
          {advanced && (
            <div className="advanced-fields">
              {field('text_base_url', '文本服务地址')}
              {field('asr_resource_id', '语音识别资源 ID')}
              {field('asr_url', '语音识别 WebSocket 地址')}
              {field('tts_resource_id', '语音合成资源 ID')}
              {field('tts_voice', '音色 ID')}
              {field('tts_url', '语音合成 HTTP 地址')}
            </div>
          )}
        </section>
        <div className="settings-footer">
          <span>
            <ShieldCheck size={15} />
            凭据仅保存在本机，不会写入会话记录。
          </span>
          <button className="button primary" disabled={saving || !!testing} type="submit">
            {saving ? <LoaderCircle size={16} className="spin" /> : <Check size={16} />}保存设置
          </button>
        </div>
      </form>
    </div>
  );
}
