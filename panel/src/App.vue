<script setup lang="ts">
import { ref, computed, watch, onMounted, onUnmounted, nextTick } from "vue";
import Icon from "./Icon.vue";
import { call, invoke, listen } from "./bridge";

type Source = "Clipboard" | "Transfer";
type Entry = {
  id: string;
  source: Source;
  timestamp: number;
  favorite: boolean;
  title: string;
  preview: string;
  kind: string;
  state: string;
  items: unknown[];
  errors: string[];
  origin?: string;
};
type Config = {
  name: string;
  level: string;
  affinity: number;
  feeds: number;
  interactions: number;
  allow_cursor_chase: boolean;
  allow_hide_icons: boolean;
  quiet: boolean;
  roaming: boolean;
  low_power: boolean;
  scale: number;
  neglect_seconds: number;
  icon_hide_seconds: number;
  clipboard_enabled: boolean;
  clipboard_days: number;
  clipboard_limit: number;
  content_quota_gib: number;
  download_dropped_resources: boolean;
};
type Snapshot = {
  config: Config;
  mood: string;
  quiet: boolean;
  busy_seconds: number;
  hidden: boolean;
  journal: string[];
  skin: { id: string; name: string; author: string; version: number };
  skin_path: string;
  status: string | null;
  content: {
    records: {
      clipboard: Entry[];
      transfer: Entry[];
      bytes: number;
      export_bytes: number;
    };
    progress: Record<string, [string, number]>;
    access: string | null;
    message: string | null;
    selected: string[];
  };
  page: number;
  platform: string;
  data_dir: string;
  version: string;
};
type Prepared = { files: string[]; payloads: unknown[] };
const pages = [
  {
    name: "一起陪伴",
    icon: "home",
    title: "今天也有我陪着你",
    subtitle: "留一点时间，给桌面上的小伙伴。",
  },
  {
    name: "偏好设置",
    icon: "settings",
    title: "按你的习惯来",
    subtitle: "让陪伴刚刚好，不多打扰。",
  },
  {
    name: "互动日记",
    icon: "journal",
    title: "我们的小小日常",
    subtitle: "那些发生在桌面上的可爱瞬间。",
  },
  {
    name: "皮肤外观",
    icon: "skin",
    title: "换一个喜欢的样子",
    subtitle: "让桌面上的它，更像你的专属伙伴。",
  },
  {
    name: "剪切板",
    icon: "clipboard",
    title: "复制过的，都在这里",
    subtitle: "文字、图片与文件，随时找回来。",
  },
  {
    name: "中转站",
    icon: "transfer",
    title: "东西先交给我",
    subtitle: "暂存想留住的内容，再带到需要它的地方。",
  },
];
const snapshot = ref<Snapshot | null>(null),
  page = ref(0),
  avatar = ref(""),
  toast = ref(""),
  fatal = ref(""),
  nameDraft = ref(""),
  skinPath = ref("");
const query = ref(""),
  filter = ref(""),
  onlyFavorites = ref(false),
  searchResults = ref<Entry[] | null>(null),
  selected = ref<string[]>([]);
const preview = ref<{ text?: string; image?: string }>({}),
  prepared = ref<Prepared | null>(null),
  preparing = ref(false),
  zoom = ref(false),
  dropHover = ref(false);
const consent = ref(false),
  acknowledged = ref(false),
  confirmClear = ref(false),
  skinZone = ref<HTMLElement | null>(null);
const source = computed<Source>(() =>
  page.value === 4 ? "Clipboard" : "Transfer",
);
const records = computed(() =>
  source.value === "Clipboard"
    ? (snapshot.value?.content.records.clipboard ?? [])
    : (snapshot.value?.content.records.transfer ?? []),
);
const shown = computed(() =>
  (query.value && searchResults.value === null
    ? []
    : (searchResults.value ?? records.value)
  ).filter(
    (e) =>
      (!filter.value || e.kind === filter.value) &&
      (!onlyFavorites.value || e.favorite),
  ),
);
const current = computed(() =>
  records.value.find((e) => selected.value.includes(e.id)),
);
const canTake = computed(
  () =>
    selected.value.length > 0 &&
    selected.value.every((id) =>
      records.value.some((e) => e.id === id && e.items.length > 0),
    ),
);
const tasks = computed(() =>
  Object.entries(snapshot.value?.content.progress ?? {}),
);
const moods: Record<string, string> = {
  Idle: "今天也想陪着你",
  Happy: "心情很好，想贴贴",
  Grumpy: "有一点点小脾气",
  Sleep: "正在闭眼休息",
  Dragged: "被你拎起来了",
  Play: "玩得很开心",
  Chase: "在追逐光标",
  Hiding: "在和你捉迷藏",
  Dizzy: "有一点晕乎乎",
  Walk: "正在悠闲散步",
};
const kindLabels: Record<string, string> = {
  text: "文本",
  link: "链接",
  image: "图片",
  files: "文件",
  mixed: "混合",
};
const kindIcons: Record<string, string> = {
  text: "text",
  link: "link",
  image: "image",
  files: "file",
  mixed: "transfer",
};
let timer: ReturnType<typeof setTimeout> | undefined,
  toastTimer: ReturnType<typeof setTimeout> | undefined,
  searchTimer: ReturnType<typeof setTimeout> | undefined;
let previewGeneration = 0,
  searchGeneration = 0,
  stopped = false,
  isDragging = false,
  lastAvatar = "",
  lastMessage = "",
  observer: ResizeObserver | undefined;
const unlisteners: Array<() => void> = [];
function showToast(value: string) {
  toast.value = value;
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => (toast.value = ""), 5000);
}
async function act(
  command: string,
  args: Record<string, unknown> = {},
  message = "",
) {
  try {
    const result = await call(command, args);
    if (message) showToast(message);
    await refresh();
    return result;
  } catch (error) {
    showToast(String(error));
    return null;
  }
}
async function refresh() {
  if (stopped) return;
  try {
    const value = await call<Snapshot>("snapshot");
    snapshot.value = value;
    fatal.value = "";
    const key = JSON.stringify([value.skin.id, value.skin_path]);
    if (key !== lastAvatar) {
      lastAvatar = key;
      avatar.value = await call<string>("skin_preview");
    }
    const message = value.content.message;
    if (message && message !== lastMessage) {
      lastMessage = message;
      showToast(message);
    }
    if (
      (page.value === 4 || page.value === 5) &&
      selected.value.length === 0 &&
      shown.value.length
    ) {
      selected.value = [shown.value[0].id];
    }
  } catch (error) {
    fatal.value = String(error);
  }
}
async function poll() {
  await refresh();
  if (!stopped) timer = setTimeout(poll, 900);
}
async function navigate(next: number) {
  if (page.value === 1 && next !== 1) await saveName();
  page.value = next;
  selected.value = [];
  query.value = "";
  filter.value = "";
  onlyFavorites.value = false;
  searchResults.value = null;
  prepared.value = null;
  if (next === 1) nameDraft.value = snapshot.value?.config.name ?? "";
  if (next === 3) skinPath.value = snapshot.value?.skin_path ?? "";
  await call("page", { page: next });
  if ((next === 4 || next === 5) && shown.value.length) {
    selected.value = [shown.value[0].id];
  }
  await nextTick();
  await registerZone();
}
async function setting(key: string, value: unknown) {
  const config = await act("update_config", { [key]: value });
  if (key === "name" && config) nameDraft.value = (config as Config).name;
}
function value(event: Event) {
  return (event.target as HTMLInputElement).value;
}
function checked(event: Event) {
  return (event.target as HTMLInputElement).checked;
}
async function saveName() {
  if (!nameDraft.value.trim()) {
    nameDraft.value = snapshot.value?.config.name ?? "";
    return;
  }
  if (nameDraft.value.trim() === snapshot.value?.config.name) return;
  await setting("name", nameDraft.value);
}
function commitNameOnEnter(event: KeyboardEvent) {
  if (!event.isComposing) void saveName();
}
async function close() {
  if (page.value === 1) await saveName();
  await invoke("close_panel");
}
function bytes(size: number) {
  return size >= 1073741824
    ? `${(size / 1073741824).toFixed(1)} GiB`
    : size >= 1048576
      ? `${(size / 1048576).toFixed(1)} MB`
      : size >= 1024
        ? `${(size / 1024).toFixed(1)} KB`
        : `${size} B`;
}
function age(stamp: number) {
  const s = Math.max(0, Date.now() / 1000 - stamp);
  return s < 60
    ? "刚刚"
    : s < 3600
      ? `${Math.floor(s / 60)} 分钟前`
      : s < 86400
        ? `${Math.floor(s / 3600)} 小时前`
        : new Date(stamp * 1000).toLocaleDateString("zh-CN");
}
function select(entry: Entry, event: MouseEvent) {
  if (event.ctrlKey || event.metaKey || event.shiftKey) {
    selected.value = selected.value.includes(entry.id)
      ? selected.value.filter((id) => id !== entry.id)
      : [...selected.value, entry.id];
  } else selected.value = [entry.id];
}
async function loadPreview() {
  const generation = ++previewGeneration;
  preview.value = {};
  prepared.value = null;
  zoom.value = false;
  const entry = current.value;
  if (!entry) return;
  try {
    const result = await call<{ text?: string; image?: string }>("preview", {
      id: entry.id,
    });
    if (generation === previewGeneration) preview.value = result;
  } catch (error) {
    if (generation === previewGeneration) showToast(String(error));
  }
}
watch(() => selected.value.join("|"), loadPreview);
watch([filter, onlyFavorites, query], () => {
  selected.value = [];
});
watch(query, () => {
  clearTimeout(searchTimer);
  const generation = ++searchGeneration;
  searchResults.value = null;
  if (!query.value) return;
  searchTimer = setTimeout(async () => {
    try {
      const entries = await call<Entry[]>("search", {
        source: source.value,
        query: query.value,
      });
      if (generation === searchGeneration) searchResults.value = entries;
    } catch (error) {
      showToast(String(error));
    }
  }, 180);
});
async function prepare() {
  if (preparing.value || !canTake.value) return;
  preparing.value = true;
  const key = selected.value.join("|");
  try {
    const result = await call<Prepared>("prepare", { ids: selected.value });
    if (key === selected.value.join("|")) prepared.value = result;
  } catch (error) {
    showToast(String(error));
  } finally {
    preparing.value = false;
  }
}
async function copy() {
  await act("copy", { ids: selected.value }, "已复制，可以到其他应用粘贴。");
}
async function save() {
  try {
    const result = await invoke("save_content", { ids: selected.value });
    if (result) showToast("正在保存到所选文件夹…");
  } catch (error) {
    showToast(String(error));
  }
}
async function startDrag(event: PointerEvent) {
  if (event.button !== 0 || !prepared.value) return;
  const x = event.clientX,
    y = event.clientY;
  const data = prepared.value;
  const move = async (e: PointerEvent) => {
    if (isDragging || Math.hypot(e.clientX - x, e.clientY - y) < 5) return;
    cleanup();
    isDragging = true;
    try {
      await invoke("drag_content", data as unknown as Record<string, unknown>);
    } catch (error) {
      isDragging = false;
      showToast(String(error));
    }
  };
  const cleanup = () => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", cleanup);
  };
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", cleanup);
}
async function deleteSelected() {
  await act("delete", { ids: selected.value });
  selected.value = [];
}
async function loadSkin(path = skinPath.value) {
  if (!path.trim()) return;
  const result = await act(
    "load_skin",
    { path: path.trim().replace(/^"|"$/g, "") },
    "新皮肤已经换好啦。",
  );
  if (result) {
    skinPath.value = snapshot.value?.skin_path ?? "";
    avatar.value = await call<string>("skin_preview");
  }
}
async function chooseSkin(folder = false) {
  try {
    const path = await invoke<string | null>("choose_skin", { folder });
    if (path) {
      skinPath.value = path;
      await loadSkin(path);
    }
  } catch (error) {
    showToast(String(error));
  }
}
async function reloadSkin() {
  if (await act("reload_skin", {}, "皮肤已重新加载。"))
    avatar.value = await call<string>("skin_preview");
}
async function defaultSkin() {
  if (await act("default_skin", {}, "已恢复默认皮肤。")) {
    skinPath.value = "";
    avatar.value = await call<string>("skin_preview");
  }
}
async function registerZone() {
  const rect =
    page.value === 3 ? skinZone.value?.getBoundingClientRect() : undefined;
  await invoke("skin_zone", {
    rect: rect ? [rect.x, rect.y, rect.width, rect.height] : null,
  }).catch(() => {});
}
async function addFiles(folder = false) {
  try {
    await invoke("add_files", { folder });
    await refresh();
  } catch (error) {
    showToast(String(error));
  }
}
async function paste() {
  try {
    await invoke("paste_current");
    showToast("正在收纳剪切板内容…");
  } catch (error) {
    showToast(String(error));
  }
}
async function cancel(id: string) {
  try {
    await invoke("cancel_task", { id });
  } catch (error) {
    showToast(String(error));
  }
}
async function enableIcons() {
  if (!acknowledged.value) return;
  if (await act("consent_icons", { acknowledged: true })) {
    consent.value = false;
    showToast("已授权，可随时停止并恢复桌面。");
  }
}
onMounted(async () => {
  try {
    const context = await invoke<{ page: number; capture: boolean }>("context");
    page.value = context.page;
    unlisteners.push(
      await listen<{ page: number; selected?: string[] }>(
        "navigate",
        async (event) => {
          await navigate(event.page);
          if (event.selected?.length) selected.value = event.selected;
        },
      ),
    );
    unlisteners.push(
      await listen<{ path: string }>("skin-drop", (event) => {
        skinPath.value = event.path;
        void loadSkin(event.path);
      }),
    );
    unlisteners.push(
      await listen<boolean>("drop-hover", (v) => (dropHover.value = v)),
    );
    unlisteners.push(
      await listen<boolean>("drag-finished", () => {
        isDragging = false;
      }),
    );
    unlisteners.push(await listen<string>("native-error", showToast));
    unlisteners.push(await listen<void>("close-requested", () => void close()));
    await refresh();
    nameDraft.value = snapshot.value?.config.name ?? "";
    skinPath.value = snapshot.value?.skin_path ?? "";
    if (snapshot.value?.content.selected.length)
      selected.value = [...snapshot.value.content.selected];
    await nextTick();
    observer = new ResizeObserver(() => void registerZone());
    if (skinZone.value) observer.observe(skinZone.value);
    window.addEventListener("resize", registerZone);
    window.addEventListener("scroll", registerZone, true);
    void poll();
    if (context.capture) {
      await document.fonts.ready;
      setTimeout(
        () => void invoke("capture").catch((e) => showToast(String(e))),
        1200,
      );
    }
  } catch (error) {
    fatal.value = String(error);
  }
});
onUnmounted(() => {
  stopped = true;
  clearTimeout(timer);
  clearTimeout(toastTimer);
  clearTimeout(searchTimer);
  observer?.disconnect();
  unlisteners.forEach((fn) => fn());
  window.removeEventListener("resize", registerZone);
  window.removeEventListener("scroll", registerZone, true);
});
</script>

<template>
  <div class="app-shell" :class="{ 'drop-active': dropHover }">
    <aside class="sidebar">
      <div class="brand">
        <div class="brand-mark"><Icon name="sparkles" :size="25" /></div>
        <div>
          <strong>mochi<span>·</span></strong
          ><small>桌面上的小伙伴</small>
        </div>
      </div>
      <div class="nav-label">我的陪伴空间</div>
      <nav aria-label="面板导航">
        <button
          v-for="(item, index) in pages"
          :key="item.name"
          :class="{ active: page === index }"
          :aria-current="page === index ? 'page' : undefined"
          @click="navigate(index)"
        >
          <Icon :name="item.icon" /><span>{{ item.name }}</span
          ><span
            v-if="index === 5 && snapshot?.content.records.transfer.length"
            class="nav-count"
            >{{ snapshot.content.records.transfer.length }}</span
          >
        </button>
      </nav>
      <div class="sidebar-pet" v-if="avatar">
        <img :src="avatar" alt="你的桌宠" />
        <p>{{ snapshot?.config.name }}在这里陪着你</p>
        <span><i class="status-dot" /> 原生宠物正在运行</span>
      </div>
      <div class="sidebar-footer">
        <Icon name="shield" :size="15" /><span>本地保存 · 不上传</span
        ><small>v{{ snapshot?.version ?? "0.1.0" }}</small>
      </div>
    </aside>
    <section class="workspace">
      <header class="page-header">
        <div>
          <span class="eyebrow">{{ pages[page]?.name }} / MOCHI</span>
          <h1>{{ pages[page]?.title }}</h1>
          <p>{{ pages[page]?.subtitle }}</p>
        </div>
        <button class="button quiet" @click="close">
          <Icon name="close" :size="16" /> 收起面板
        </button>
      </header>
      <main class="page-body" @scroll="registerZone">
        <div v-if="fatal" class="alert error">
          <Icon name="shield" />{{ fatal }}
        </div>
        <div v-if="snapshot?.status" class="alert subtle">
          {{ snapshot.status }}
        </div>
        <template v-if="snapshot">
          <template v-if="page === 0">
            <div class="hero-card">
              <div class="hero-copy">
                <span class="pill warm"
                  ><Icon name="heart" :size="14" /> 你的专属伙伴</span
                >
                <h2>
                  你好呀，<br />我是{{ snapshot.config.name
                  }}<span class="wave">✦</span>
                </h2>
                <p>
                  {{
                    moods[snapshot.mood] ?? "今天也想陪着你"
                  }}。<br />一起把普通的日子，过得可爱一点。
                </p>
                <button
                  class="button primary"
                  @click="act('interact', { action: 'pet' }, '已经摸摸它啦。')"
                >
                  <Icon name="heart" :size="17" /> 摸摸头
                </button>
              </div>
              <div class="hero-art">
                <div class="orbit one" />
                <div class="orbit two" />
                <img v-if="avatar" :src="avatar" alt="当前桌宠" /><span
                  class="hero-bubble"
                  >{{
                    snapshot.quiet ? "我会安静陪着你。" : "今天也请多关照呀！"
                  }}</span
                >
              </div>
            </div>
            <div class="stats-grid">
              <div class="stat-card">
                <span><Icon name="heart" />亲密度</span
                ><strong
                  >{{ snapshot.config.affinity }}<small>/ 100</small></strong
                >
                <div class="meter">
                  <i :style="{ width: snapshot.config.affinity + '%' }" />
                </div>
              </div>
              <div class="stat-card">
                <span><Icon name="sparkles" />一起互动</span
                ><strong
                  >{{ snapshot.config.interactions }}<small>次</small></strong
                >
                <p>每一次靠近，它都记得</p>
              </div>
              <div class="stat-card">
                <span><Icon name="play" />投喂的小饼干</span
                ><strong>{{ snapshot.config.feeds }}<small>颗</small></strong>
                <p>把快乐攒成满满一口袋</p>
              </div>
            </div>
            <div class="section-title">
              <h3>今天想做点什么？</h3>
              <span>小小的互动，也会让它开心</span>
            </div>
            <div class="interaction-grid">
              <button @click="act('interact', { action: 'feed' })">
                <span class="interaction-icon peach"
                  ><Icon name="heart" :size="23" /></span
                ><strong>投喂一块饼干</strong><small>分享一点甜甜的快乐</small
                ><Icon name="arrow" /></button
              ><button @click="act('interact', { action: 'play' })">
                <span class="interaction-icon lilac"
                  ><Icon name="play" :size="23" /></span
                ><strong>一起玩球</strong><small>让它蹦蹦跳跳起来</small
                ><Icon name="arrow" /></button
              ><button @click="act('interact', { action: 'busy' })">
                <span class="interaction-icon sage"
                  ><Icon name="moon" :size="23" /></span
                ><strong>我在忙</strong><small>安静陪伴 30 分钟</small
                ><Icon name="arrow" />
              </button>
            </div>
            <div class="card compact">
              <div class="inline-row">
                <div>
                  <h3>按自己的节奏相处</h3>
                  <p>
                    {{
                      snapshot.busy_seconds
                        ? `忙碌陪伴还剩 ${Math.ceil(snapshot.busy_seconds / 60)} 分钟`
                        : "左键摸摸 · 双击戳戳 · 拖动拎起来"
                    }}
                  </p>
                </div>
                <div class="actions">
                  <button
                    class="button"
                    @click="act('interact', { action: 'wake' })"
                  >
                    叫醒它</button
                  ><button
                    class="button"
                    @click="act('interact', { action: 'home' })"
                  >
                    回到角落
                  </button>
                </div>
              </div>
              <div class="divider" />
              <div class="actions">
                <button
                  class="button quiet"
                  :disabled="
                    !snapshot.config.allow_cursor_chase || snapshot.quiet
                  "
                  @click="act('interact', { action: 'chase' })"
                >
                  体验光标追逐</button
                ><button
                  class="button quiet"
                  :disabled="
                    !snapshot.config.allow_hide_icons ||
                    snapshot.config.level !== 'Devil' ||
                    snapshot.quiet ||
                    snapshot.hidden
                  "
                  @click="act('interact', { action: 'hide' })"
                >
                  体验图标捉迷藏</button
                ><small>高级互动需要在偏好设置中单独授权。</small>
              </div>
            </div>
          </template>
          <template v-else-if="page === 1">
            <section class="card">
              <div class="section-title">
                <h3>基本信息</h3>
                <span>一个只属于它的名字</span>
              </div>
              <label class="field-label" for="nickname">昵称</label>
              <div class="input-with-action">
                <input
                  id="nickname"
                  v-model="nameDraft"
                  placeholder="例如：曦玥、团团"
                  @keydown.enter="commitNameOnEnter"
                  @blur="saveName"
                /><button class="button primary" @click="saveName">
                  保存昵称
                </button>
              </div>
              <p class="field-help">
                支持中文输入 · 最多 16 个字符
                <span>{{ Array.from(nameDraft).length }} / 16</span>
              </p>
            </section>
            <section class="card">
              <div class="section-title">
                <h3>陪伴方式</h3>
                <span>它可以调皮，也可以很懂事</span>
              </div>
              <div class="choice-grid">
                <button
                  v-for="item in [
                    {
                      value: 'Calm',
                      title: '乖巧',
                      description: '安静温柔，不主动捣蛋',
                    },
                    {
                      value: 'Playful',
                      title: '调皮',
                      description: '偶尔撒娇，带一点小惊喜',
                    },
                    {
                      value: 'Devil',
                      title: '小恶魔',
                      description: '更爱争宠，高级捣蛋需授权',
                    },
                  ]"
                  :key="item.value"
                  :class="{ selected: snapshot.config.level === item.value }"
                  @click="setting('level', item.value)"
                >
                  <strong>{{ item.title }}</strong>
                  <p>{{ item.description }}</p>
                  <Icon
                    v-if="snapshot.config.level === item.value"
                    name="check"
                    :size="17"
                  />
                </button>
              </div>
              <div
                v-for="item in [
                  {
                    key: 'quiet',
                    title: '安静陪伴',
                    description: '不主动捣蛋，专心陪你工作。',
                  },
                  {
                    key: 'roaming',
                    title: '允许散步',
                    description: '在当前屏幕的工作区内悠闲走动。',
                  },
                  {
                    key: 'low_power',
                    title: '省电模式',
                    description: '活动动画限制为 4 FPS。',
                  },
                ]"
                :key="item.key"
                class="setting-row"
              >
                <div>
                  <strong>{{ item.title }}</strong>
                  <p>{{ item.description }}</p>
                </div>
                <label class="switch"
                  ><input
                    type="checkbox"
                    :aria-label="item.title"
                    :checked="
                      Boolean(snapshot.config[item.key as keyof Config])
                    "
                    @change="setting(item.key, checked($event))" /><span
                /></label>
              </div>
              <div class="slider-row">
                <label for="scale">宠物大小</label
                ><input
                  id="scale"
                  type="range"
                  min="0.65"
                  max="1.5"
                  step="0.05"
                  :value="snapshot.config.scale"
                  @change="setting('scale', Number(value($event)))"
                /><span>{{ Math.round(snapshot.config.scale * 100) }}%</span>
              </div>
              <div class="slider-row">
                <label for="neglect">争宠等待</label
                ><input
                  id="neglect"
                  type="range"
                  min="1"
                  max="120"
                  :value="Math.round(snapshot.config.neglect_seconds / 60)"
                  @change="
                    setting('neglect_seconds', Number(value($event)) * 60)
                  "
                /><span
                  >{{
                    Math.round(snapshot.config.neglect_seconds / 60)
                  }}
                  分钟</span
                >
              </div>
            </section>
            <section class="card">
              <div class="section-title">
                <h3>互动与授权</h3>
                <span>你的桌面，你来决定</span>
              </div>
              <div class="setting-row">
                <div>
                  <strong>允许追逐光标</strong>
                  <p>仅做视觉互动，不移动鼠标、不拦截点击。</p>
                </div>
                <label class="switch"
                  ><input
                    type="checkbox"
                    aria-label="允许追逐光标"
                    :checked="snapshot.config.allow_cursor_chase"
                    @change="
                      setting('allow_cursor_chase', checked($event))
                    " /><span
                /></label>
              </div>
              <div class="setting-row">
                <div>
                  <strong>桌面图标捉迷藏</strong>
                  <p>高级捣蛋需单独授权，可随时恢复。</p>
                </div>
                <button
                  class="button"
                  @click="
                    snapshot.config.allow_hide_icons
                      ? setting('allow_hide_icons', false)
                      : ((consent = true), (acknowledged = false))
                  "
                >
                  {{
                    snapshot.config.allow_hide_icons ? "撤销授权" : "了解并授权"
                  }}
                </button>
              </div>
              <div class="slider-row">
                <label for="hide-seconds">自动归还</label
                ><input
                  id="hide-seconds"
                  type="range"
                  min="5"
                  max="30"
                  :value="snapshot.config.icon_hide_seconds"
                  @change="setting('icon_hide_seconds', Number(value($event)))"
                /><span>{{ snapshot.config.icon_hide_seconds }} 秒</span>
              </div>
            </section>
            <section class="card">
              <div class="section-title">
                <h3>剪切板与内容库</h3>
                <span>你收藏的东西，会好好留着</span>
              </div>
              <div class="setting-row">
                <div>
                  <strong>记录剪切板</strong>
                  <p>只记录启动后新复制的内容，暂停期间不补录。</p>
                </div>
                <label class="switch"
                  ><input
                    type="checkbox"
                    aria-label="记录剪切板"
                    :checked="snapshot.config.clipboard_enabled"
                    @change="
                      setting('clipboard_enabled', checked($event))
                    " /><span
                /></label>
              </div>
              <div class="number-grid">
                <label
                  >保留天数<input
                    type="number"
                    min="1"
                    max="3650"
                    :value="snapshot.config.clipboard_days"
                    @change="setting('clipboard_days', Number(value($event)))"
                  /><small>天 · 收藏除外</small></label
                ><label
                  >非收藏记录上限<input
                    type="number"
                    min="10"
                    max="100000"
                    :value="snapshot.config.clipboard_limit"
                    @change="setting('clipboard_limit', Number(value($event)))"
                  /><small>条</small></label
                ><label
                  >内容库空间<input
                    type="number"
                    min="1"
                    max="1024"
                    :value="snapshot.config.content_quota_gib"
                    @change="
                      setting('content_quota_gib', Number(value($event)))
                    "
                  /><small>GiB</small></label
                >
              </div>
              <div class="setting-row">
                <div>
                  <strong>下载拖入的网络资源</strong>
                  <p>明确的图片或文件可下载；普通网页链接只作记录。</p>
                </div>
                <label class="switch"
                  ><input
                    type="checkbox"
                    aria-label="下载网络资源"
                    :checked="snapshot.config.download_dropped_resources"
                    @change="
                      setting('download_dropped_resources', checked($event))
                    " /><span
                /></label>
              </div>
              <p class="field-help">
                收藏和中转内容不会自动删除。仅收纳远程资源时下载，无上传。
              </p>
            </section>
          </template>
          <template v-else-if="page === 2"
            ><section class="card journal-intro">
              <span class="interaction-icon peach"
                ><Icon name="journal" :size="27"
              /></span>
              <div>
                <h2>每一次互动，都是小小的回忆</h2>
                <p>
                  本次陪伴的最近 {{ snapshot.journal.length }} 条日记 ·
                  退出后重新开始
                </p>
              </div>
            </section>
            <div class="timeline">
              <div
                v-for="(line, index) in [...snapshot.journal].reverse()"
                :key="index"
                class="timeline-item"
              >
                <span class="timeline-dot" />
                <div>
                  <span class="eyebrow"
                    >{{ index === 0 ? "刚刚的回忆" : "一起陪伴" }} ·
                    {{ snapshot.config.name }}</span
                  >
                  <p>{{ line }}</p>
                </div>
              </div>
            </div>
            <div class="empty-note" v-if="!snapshot.journal.length">
              日记还很空，去和它打个招呼吧。
            </div></template
          >
          <template v-else-if="page === 3"
            ><section class="skin-hero card">
              <div class="skin-stage">
                <img v-if="avatar" :src="avatar" alt="当前皮肤预览" />
              </div>
              <div>
                <span class="pill warm">当前使用</span>
                <h2>{{ snapshot.skin.name }}</h2>
                <p>
                  {{
                    snapshot.skin.author
                      ? "作者 · " + snapshot.skin.author
                      : "内置伙伴"
                  }}<span class="muted">
                    · 版本 {{ snapshot.skin.version }}</span
                  >
                </p>
                <p class="skin-description">
                  造型、表情与动画，<br />都可以换成你喜欢的样子。
                </p>
                <div class="actions">
                  <button
                    class="button"
                    :disabled="!snapshot.skin_path"
                    @click="reloadSkin"
                  >
                    <Icon name="refresh" :size="16" />重新加载</button
                  ><button class="button quiet" @click="defaultSkin">
                    恢复默认
                  </button>
                </div>
              </div>
            </section>
            <section class="card">
              <div class="section-title">
                <h3>给它换一套新皮肤</h3>
                <span>.mochi / ZIP / 皮肤目录</span>
              </div>
              <div ref="skinZone" class="skin-drop-zone">
                <Icon name="skin" :size="29" /><strong>把皮肤包拖到这里</strong>
                <p>此区域专用于换皮肤，其他区域用于中转收纳。</p>
                <div class="actions">
                  <button class="button primary" @click="chooseSkin(false)">
                    选择皮肤包</button
                  ><button class="button" @click="chooseSkin(true)">
                    选择皮肤目录
                  </button>
                </div>
              </div>
              <label class="field-label" for="skin-path">或粘贴完整路径</label>
              <div class="input-with-action">
                <input
                  id="skin-path"
                  v-model="skinPath"
                  placeholder="皮肤包或目录的完整路径"
                  @keydown.enter="loadSkin()"
                /><button class="button" @click="loadSkin()">加载并使用</button>
              </div>
              <p class="field-help">
                修改目录里的 UI 或动画后，重新加载即可查看变化。
              </p>
            </section></template
          >
          <template v-else>
            <section class="content-toolbar card">
              <div class="inline-row">
                <div class="content-status" v-if="page === 4">
                  <span
                    class="status-pill"
                    :class="{
                      paused: !snapshot.config.clipboard_enabled,
                      restricted: !!snapshot.content.access,
                    }"
                    ><i class="status-dot" />{{
                      !snapshot.config.clipboard_enabled
                        ? "已暂停"
                        : snapshot.content.access
                          ? "访问受限"
                          : "正在记录"
                    }}</span
                  ><span
                    >{{
                      snapshot.content.records.clipboard.length
                    }}
                    条历史</span
                  >
                </div>
                <div class="content-status" v-else>
                  <span class="status-pill"
                    ><Icon name="transfer" :size="16" />{{
                      snapshot.content.records.transfer.length
                    }}
                    条记录</span
                  ><span
                    >{{ bytes(snapshot.content.records.bytes) }} /
                    {{ snapshot.config.content_quota_gib }} GiB</span
                  >
                </div>
                <div class="actions" v-if="page === 4">
                  <button
                    class="button"
                    @click="
                      setting(
                        'clipboard_enabled',
                        !snapshot.config.clipboard_enabled,
                      )
                    "
                  >
                    <Icon
                      :name="
                        snapshot.config.clipboard_enabled ? 'pause' : 'play'
                      "
                      :size="16"
                    />{{
                      snapshot.config.clipboard_enabled
                        ? "暂停记录"
                        : "开始记录"
                    }}</button
                  ><button class="button quiet" @click="confirmClear = true">
                    清空历史
                  </button>
                </div>
                <div class="actions" v-else>
                  <button class="button primary" @click="addFiles(false)">
                    <Icon name="plus" :size="16" />添加文件</button
                  ><button class="button" @click="addFiles(true)">
                    添加目录</button
                  ><button class="button quiet" @click="paste">
                    粘贴剪切板
                  </button>
                </div>
              </div>
              <p class="field-help">
                {{
                  page === 4
                    ? `自动保留 ${snapshot.config.clipboard_days} 天 / 最近 ${snapshot.config.clipboard_limit} 条非收藏记录 · 文件仅记录原路径`
                    : `拖到宠物上即可收纳 · 原文件不移动、不删除 · 导出缓存 ${bytes(snapshot.content.records.export_bytes)}`
                }}
              </p>
              <div v-if="snapshot.content.access" class="alert error">
                {{ snapshot.content.access }}
              </div>
              <div v-for="[id, task] in tasks" :key="id" class="task-row">
                <span class="spinner" /><span
                  >{{ task[0] }} · {{ bytes(task[1]) }}</span
                ><button class="button quiet small" @click="cancel(id)">
                  取消
                </button>
              </div>
            </section>
            <div class="filter-bar">
              <label class="search"
                ><Icon name="search" :size="19" /><input
                  v-model="query"
                  placeholder="搜索文字、链接或文件名"
                  aria-label="搜索记录" /></label
              ><select v-model="filter" aria-label="内容类型">
                <option value="">全部类型</option>
                <option
                  v-for="(label, key) in kindLabels"
                  :key="key"
                  :value="key"
                >
                  {{ label }}
                </option></select
              ><label class="favorite-filter"
                ><input type="checkbox" v-model="onlyFavorites" /><Icon
                  name="star"
                  :size="15"
                />只看收藏</label
              >
            </div>
            <div v-if="!shown.length" class="empty-state">
              <span class="empty-icon"
                ><Icon :name="page === 4 ? 'clipboard' : 'transfer'" :size="39"
              /></span>
              <h3>
                {{
                  query
                    ? "没有找到匹配内容"
                    : page === 4
                      ? "复制的内容，会在这里留下记录"
                      : "把想暂存的东西，交给桌宠吧"
                }}
              </h3>
              <p>
                {{
                  query
                    ? "换一个关键词试试看。"
                    : "文字、图片、文件和附件，都可以在这里找回。"
                }}
              </p>
            </div>
            <div v-else class="content-split">
              <section class="record-list" aria-label="内容记录">
                <button
                  v-for="entry in shown"
                  :key="entry.id"
                  class="record"
                  :class="{ selected: selected.includes(entry.id) }"
                  @click="select(entry, $event)"
                >
                  <span class="record-icon" :class="entry.kind"
                    ><Icon
                      :name="kindIcons[entry.kind] ?? 'file'"
                      :size="21" /></span
                  ><span class="record-copy"
                    ><strong>{{ entry.title }}</strong
                    ><small
                      >{{ kindLabels[entry.kind] }} · {{ age(entry.timestamp)
                      }}<span v-if="entry.state !== 'Ready'">
                        ·
                        {{
                          entry.state === "Partial"
                            ? "部分完成"
                            : entry.state === "Failed"
                              ? "失败"
                              : "保存中"
                        }}</span
                      ></small
                    >
                    <p>{{ entry.preview }}</p></span
                  ><Icon v-if="entry.favorite" name="star" :size="14" />
                </button>
                <p class="list-help">按住 Ctrl / Command 点击可多选</p>
              </section>
              <section class="record-detail card" v-if="current">
                <div class="detail-heading">
                  <span class="pill">{{
                    selected.length > 1
                      ? `已选择 ${selected.length} 项`
                      : kindLabels[current.kind]
                  }}</span>
                  <h3>
                    {{
                      selected.length > 1 ? "把这些内容一起带走" : current.title
                    }}
                  </h3>
                  <p>
                    {{
                      new Date(current.timestamp * 1000).toLocaleString("zh-CN")
                    }}
                  </p>
                </div>
                <div class="detail-actions">
                  <button
                    class="button primary"
                    :disabled="!canTake"
                    @click="copy"
                  >
                    <Icon name="copy" :size="15" />复制</button
                  ><button class="button" :disabled="!canTake" @click="save">
                    另存到</button
                  ><button
                    class="icon-button"
                    :class="{ favorited: current.favorite }"
                    :aria-label="current.favorite ? '取消收藏' : '收藏'"
                    @click="
                      act('favorite', {
                        ids: selected,
                        value: !current.favorite,
                      })
                    "
                  >
                    <Icon name="star" :size="18" /></button
                  ><button
                    class="icon-button"
                    aria-label="删除记录"
                    @click="deleteSelected"
                  >
                    <Icon name="trash" :size="18" />
                  </button>
                </div>
                <button
                  v-if="page === 4"
                  class="collect-button"
                  :disabled="!canTake"
                  @click="
                    act('collect', { ids: selected }, '正在保存独立副本…')
                  "
                >
                  <Icon name="transfer" :size="17" /> 收纳到中转站
                  <Icon name="arrow" :size="16" />
                </button>
                <div class="divider" />
                <button
                  v-if="!prepared"
                  class="prepare-button"
                  :disabled="!canTake || preparing"
                  @click="prepare"
                >
                  <span v-if="preparing" class="spinner" /><Icon
                    v-else
                    name="grip"
                    :size="17"
                  />{{
                    preparing ? "正在准备，请稍候…" : "准备拖到其他应用"
                  }}</button
                ><button v-else class="drag-handle" @pointerdown="startDrag">
                  <Icon name="grip" :size="20" /><span
                    >按住这里，拖到其他应用</span
                  ><Icon name="arrow" :size="17" />
                </button>
                <p class="field-help">默认复制，记录继续保留。</p>
                <div class="preview-area">
                  <button
                    v-if="preview.image"
                    class="image-preview"
                    @click="zoom = true"
                  >
                    <img :src="preview.image" alt="内容图片预览" /><span
                      >点击查看大图</span
                    >
                  </button>
                  <pre v-if="preview.text">{{ preview.text }}</pre>
                  <p v-if="!preview.text && !preview.image" class="muted">
                    {{ current.preview }}
                  </p>
                </div>
                <div
                  v-for="error in current.errors"
                  :key="error"
                  class="alert error"
                >
                  {{ error }}
                </div>
                <div
                  v-if="current.origin && current.state !== 'Ready'"
                  class="retry"
                >
                  <p>{{ current.origin }}</p>
                  <button
                    class="button small"
                    @click="
                      act('import', {
                        payloads: [
                          { Remote: { url: current.origin, name: null } },
                        ],
                      })
                    "
                  >
                    重试下载
                  </button>
                </div>
                <p
                  v-if="page === 4 && current.kind === 'files'"
                  class="field-help"
                >
                  文件历史使用原路径；收纳后才保存独立副本。
                </p>
              </section>
              <section class="card record-detail" v-else>
                <p>选择左侧内容查看详情。</p>
              </section>
            </div>
          </template>
        </template>
        <div v-else-if="!fatal" class="loading">
          <span class="spinner" /> 正在连接你的桌宠…
        </div>
      </main>
      <footer class="recovery-bar">
        <button
          class="restore-button"
          @click="act('restore', {}, '已停止捣蛋，正在恢复桌面。')"
        >
          <Icon name="shield" :size="17" />恢复桌面 / 停止捣蛋</button
        ><span>Ctrl+Alt+Shift+R</span>
        <div class="footer-end">
          <i class="status-dot" />修改自动保存在本机<button
            class="icon-button"
            aria-label="退出桌宠"
            title="退出桌宠并恢复桌面"
            @click="act('quit')"
          >
            <Icon name="power" :size="18" />
          </button>
        </div>
      </footer>
    </section>
    <div v-if="dropHover" class="drop-overlay">
      <Icon name="transfer" :size="35" /><strong>松开交给我，好好收着</strong>
      <p>皮肤请放到皮肤页的专用区域</p>
    </div>
    <div v-if="toast" class="toast" role="status">
      <Icon name="check" :size="18" />{{ toast
      }}<button class="icon-button" aria-label="关闭提示" @click="toast = ''">
        <Icon name="close" :size="15" />
      </button>
    </div>
    <div
      v-if="consent || confirmClear || zoom"
      class="modal-backdrop"
      @click.self="
        consent = false;
        confirmClear = false;
        zoom = false;
      "
    >
      <section v-if="consent" class="modal">
        <span class="interaction-icon peach"
          ><Icon name="shield" :size="27"
        /></span>
        <h2>让它和桌面图标捉迷藏？</h2>
        <p>这会临时改变系统桌面图标的可见性，不修改桌面文件。</p>
        <div class="alert subtle">
          {{
            snapshot?.platform === "macos"
              ? "macOS 会临时修改 Finder 的 CreateDesktop 并重启 Finder；Finder 窗口可能闪烁，进行中的操作可能受影响。"
              : "Windows 会暂时隐藏 Explorer 的桌面图标窗口，5–30 秒后自动归还。"
          }}
        </div>
        <p>
          恢复按钮、快捷键、托盘和正常退出都会尝试恢复；恢复失败时保留日志供重试。
        </p>
        <label class="acknowledgement"
          ><input
            type="checkbox"
            v-model="acknowledged"
          />我了解影响，并允许临时隐藏桌面图标</label
        >
        <div class="actions end">
          <button class="button" @click="consent = false">暂时不要</button
          ><button
            class="button primary"
            :disabled="!acknowledged"
            @click="enableIcons"
          >
            确认授权
          </button>
        </div>
      </section>
      <section v-else-if="confirmClear" class="modal">
        <h2>清空剪切板历史？</h2>
        <p>删除非收藏历史，收藏与中转内容继续保留。</p>
        <div class="actions end">
          <button class="button" @click="confirmClear = false">取消</button
          ><button
            class="button primary"
            @click="
              act('clear_clipboard');
              confirmClear = false;
            "
          >
            清空历史
          </button>
        </div>
      </section>
      <section v-else class="image-modal">
        <button class="icon-button" aria-label="关闭大图" @click="zoom = false">
          <Icon name="close" /></button
        ><img v-if="preview.image" :src="preview.image" alt="图片大图预览" />
      </section>
    </div>
  </div>
</template>
