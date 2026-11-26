(() => {
'use strict';
// TokenPulse Phase 3 Interactive Visual Prototype (Pure Native JS, Zero Dependencies)
// Security rule: never inject dynamic labels with innerHTML; always use textContent / createElementNS.

const I18N = {
  zh: {
    reviewTitle: 'TokenPulse · 阶段三视觉原型评审台',
    reviewSub: 'Direct2D 悬浮窗 + 按需 WebView2 详情面板 · 双端统一 Design Tokens',
    themeLabel: '主题',
    themeDark: '黑曜石 Obsidian',
    themeLight: '青瓷霜 Porcelain',
    langLabel: '语言',
    sceneLabel: '状态场景',
    sceneNormal: '正常活跃（实测风格）',
    sceneExpired: '过期等待新活动',
    sceneRestricted: '低额度 + 套餐受限',
    sceneEmpty: '首次空态',
    opacityLabel: '透明度',
    clickThrough: '点击穿透',
    secOverlayTitle: '一、Windows 11 桌面悬浮窗（Direct2D / DirectComposition）与系统托盘',
    secOverlayNote: '零帧空转 · Per-Monitor DPI V2 · 关闭详情即销毁 WebView2，常驻 < 20 MB',
    specCapsule: '紧凑胶囊态 · 244 × 46 DIP',
    specExpanded: '展开卡片态 · 332 DIP',
    specTray: '原创托盘图标套件 & 菜单',
    trayOk: '正常',
    trayWarn: '低额度',
    trayPaused: '已暂停',
    trayOffline: '离线',
    trayM1: '打开详细统计面板',
    trayM2: '切换悬浮窗（胶囊 / 展开）',
    trayM3: '解锁整窗点击穿透',
    trayM4: '立即刷新本地日志',
    trayM5: '暂停监控',
    trayM6: '退出 TokenPulse',
    secDetailTitle: '二、按需 WebView2 详细统计面板（980+ DIP 视口）',
    secDetailNote: '纯原生 HTML/CSS/ESM/SVG · 严格 CSP connect-src none · 所有模型名纯文本渲染',
    privacyOffline: '纯离线只读 · 零外网 · 零凭据',
    filterAll: '全部平台',
    rangeToday: '今日',
    range7d: '近 7 天',
    range30d: '近 30 天',
    exportJson: '导出 JSON',
    exportCsv: '导出 CSV',
    kpiTotal: '总 Token 消耗',
    kpiCache: '双维缓存命中率',
    kpiCost: 'API 等效估算',
    kpiReqs: '有效执行与覆盖度',
    latestTurn: '最近一轮',
    cumulative: '累计均值',
    notActualBill: '非订阅实付',
    pricedCoverage: '已定价覆盖率',
    trendTitle: 'Token 消耗趋势',
    heatmapTitle: '近 30 天活跃热力图（5 档）',
    less: '低',
    more: '高',
    seriesCache: '缓存读取',
    seriesInput: '普通输入',
    seriesOutput: '输出',
    seriesReason: '推理子集',
    quotasTitle: '平台额度与 Credits 状态',
    accountUnconfirmed: '最近会话观测（账号未确认）',
    agOfflineNotice: 'Antigravity 离线日志已启用（零凭据/零正文）；在线额度组 RPC 因自签证书已按安全红线保持关闭。',
    awaitingReset: '已到预计重置时间，等待新活动（绝不自动补满 100%）',
    planRestrictedPrefix: '当前套餐无权限：',
    noData: '暂无数据',
    creditsLabel: 'Credits / 套餐',
    tableTitle: '模型使用分布',
    colModel: '模型',
    colProvider: '平台',
    colReqs: '请求数',
    colInput: '普通输入',
    colCache: '缓存读取',
    colOutput: '输出 (含推理)',
    colCacheRate: '缓存命中',
    colCost: 'API 等效估算',
    unpricedLabel: '暂无公开费率',
    coverageComplete: '扫描完整',
    coveragePartial: '覆盖不足（存在缺口）',
    coverageUnknown: '等待首次扫描',
    ovOpenDetail: '详情',
    ovPin: '置顶',
    ovPass: '穿透中',
    ovZeroNet: '零外网 · 0 帧空闲',
    emptyModels: '当前时间范围内暂无记录，开始使用 Codex 或 Antigravity 后将自动增量采集。',
    footerLeft: 'TokenPulse v0.1 · 独立非官方开源实现，与 OpenAI、Google 无关联',
    footerRight: '只读本地日志 · 严禁触碰 auth.json 与对话正文 · 零额外模型 Token',
    exportToast: '已触发原生侧脱敏导出（不含绝对路径与正文）：',
    resetsIn: '重置倒计时',
    observedAt: '观测于',
    remaining: '剩余',
    used: '已用',
    codexOnlineTag: '纯离线 JSONL',
    agOnlineTag: '纯离线 Transcript',
    capToday: '今日',
  },
  en: {
    reviewTitle: 'TokenPulse · Phase 3 Visual Prototype Studio',
    reviewSub: 'Direct2D Desktop Overlay + On-Demand WebView2 Detail Panel · Shared Design Tokens',
    themeLabel: 'Theme',
    themeDark: 'Obsidian Slate',
    themeLight: 'Porcelain Frost',
    langLabel: 'Lang',
    sceneLabel: 'State',
    sceneNormal: 'Active (Realistic)',
    sceneExpired: 'Expired / Awaiting',
    sceneRestricted: 'Low + Restricted',
    sceneEmpty: 'Empty / First Run',
    opacityLabel: 'Opacity',
    clickThrough: 'Click-Through',
    secOverlayTitle: '1. Windows 11 Desktop Overlay (Direct2D / DirectComposition) & System Tray',
    secOverlayNote: 'Zero idle commits · Per-Monitor DPI V2 · WebView2 destroyed on close (< 20 MB idle)',
    specCapsule: 'Compact Capsule · 244 × 46 DIP',
    specExpanded: 'Expanded Card · 332 DIP',
    specTray: 'Original Tray Icons & Menu',
    trayOk: 'Healthy',
    trayWarn: 'Low Quota',
    trayPaused: 'Paused',
    trayOffline: 'Offline',
    trayM1: 'Open Detail Dashboard',
    trayM2: 'Toggle Overlay (Capsule / Card)',
    trayM3: 'Unlock Click-Through',
    trayM4: 'Refresh Local Logs Now',
    trayM5: 'Pause Monitoring',
    trayM6: 'Quit TokenPulse',
    secDetailTitle: '2. On-Demand WebView2 Detail Panel (980+ DIP Viewport)',
    secDetailNote: 'Pure native HTML/CSS/ESM/SVG · Strict CSP connect-src none · Text-only model rendering',
    privacyOffline: '100% Local Read-Only · Zero Network',
    filterAll: 'All Providers',
    rangeToday: 'Today',
    range7d: 'Last 7d',
    range30d: 'Last 30d',
    exportJson: 'Export JSON',
    exportCsv: 'Export CSV',
    kpiTotal: 'Total Tokens',
    kpiCache: 'Dual Cache Hit Rate',
    kpiCost: 'API-Equivalent Est.',
    kpiReqs: 'Requests & Coverage',
    latestTurn: 'Latest Turn',
    cumulative: 'Cumulative',
    notActualBill: 'Not subscription bill',
    pricedCoverage: 'Priced coverage',
    trendTitle: 'Token Usage Trend',
    heatmapTitle: '30-Day Activity Heatmap (5 Tiers)',
    less: 'Less',
    more: 'More',
    seriesCache: 'Cache Read',
    seriesInput: 'Uncached In',
    seriesOutput: 'Output',
    seriesReason: 'Reasoning',
    quotasTitle: 'Quotas & Credits Status',
    accountUnconfirmed: 'Recent local session (account unconfirmed)',
    agOfflineNotice: 'Antigravity offline transcript metrics active (zero credentials/body). Local RPC stays disabled due to self-signed cert policy.',
    awaitingReset: 'Reset window reached — awaiting new activity (never auto-filled to 100%)',
    planRestrictedPrefix: 'Plan restricted: ',
    noData: 'No Data',
    creditsLabel: 'Credits / Plan',
    tableTitle: 'Model Distribution',
    colModel: 'Model',
    colProvider: 'Provider',
    colReqs: 'Reqs',
    colInput: 'Uncached In',
    colCache: 'Cache Read',
    colOutput: 'Output (Reas.)',
    colCacheRate: 'Cache Hit',
    colCost: 'API Est.',
    unpricedLabel: 'Unpriced (no public rate)',
    coverageComplete: 'Complete',
    coveragePartial: 'Partial (gaps)',
    coverageUnknown: 'Pending scan',
    ovOpenDetail: 'Detail',
    ovPin: 'Topmost',
    ovPass: 'Pass-Thru',
    ovZeroNet: 'Zero Net · 0 Idle FPS',
    emptyModels: 'No usage recorded in this window. Activity in Codex or Antigravity will sync incrementally.',
    footerLeft: 'TokenPulse v0.1 · Independent unofficial open-source project; not affiliated with OpenAI or Google',
    footerRight: 'Read-only local logs · Never reads auth.json or prompt bodies · 0 extra model tokens',
    exportToast: 'Triggered native redacted export (zero paths or prompts): ',
    resetsIn: 'Resets in',
    observedAt: 'Observed',
    remaining: 'left',
    used: 'used',
    codexOnlineTag: 'Offline JSONL',
    agOnlineTag: 'Offline Transcript',
    capToday: 'Today',
  },
};

function makeHeatmap(seedLevels) {
  return seedLevels.map((lv, idx) => {
    const dayOffset = 29 - idx;
    const d = new Date(Date.UTC(2026, 10, 26 - dayOffset));
    const label = `${d.getUTCFullYear()}-${String(d.getUTCMonth() + 1).padStart(2, '0')}-${String(d.getUTCDate()).padStart(2, '0')}`;
    const tokens = lv === 0 ? 0 : lv * 3_800_000 + (idx % 5) * 420_000;
    const reqs = lv === 0 ? 0 : lv * 42 + (idx % 7) * 5;
    return { date: label, level: lv, tokens, reqs };
  });
}

const SCENES = {
  normal: {
    coverage: 'Complete',
    latestTurnCache: 0.974,
    codexLatestTurnCache: 0.974,
    agLatestTurnCache: 0.992,
    quotas: [
      {
        provider: 'codex',
        label: 'Codex · 5h Window',
        period: '5h',
        kind: 'active',
        usedPct: 18,
        resetsText: '3h 42m',
        observedText: '21:44',
      },
      {
        provider: 'codex',
        label: 'Codex · Weekly (7d)',
        period: '7d',
        kind: 'active',
        usedPct: 5,
        resetsText: '6d 14h',
        observedText: '21:44',
      },
    ],
    credits: {
      planType: 'prolite',
      serviceTier: 'standard',
      stateText: 'None (Included Plan)',
      observedText: '21:44',
    },
    ranges: {
      today: {
        codex: { reqs: 416, nonCached: 1_003_438, cacheRead: 19_464_960, output: 277_877, reasoning: 42_624, costUsd: 0, pricedRatio: 0 },
        antigravity: { reqs: 128, nonCached: 1_420_500, cacheRead: 18_960_200, output: 248_900, reasoning: 0, costUsd: 0, pricedRatio: 0 },
      },
      '7d': {
        codex: { reqs: 1214, nonCached: 2_552_757, cacheRead: 95_154_944, output: 583_078, reasoning: 100_020, costUsd: 14.82, pricedRatio: 0.18 },
        antigravity: { reqs: 391, nonCached: 4_175_179, cacheRead: 57_275_708, output: 769_124, reasoning: 0, costUsd: 0, pricedRatio: 0 },
      },
      '30d': {
        codex: { reqs: 1335, nonCached: 2_890_120, cacheRead: 104_590_200, output: 641_150, reasoning: 115_800, costUsd: 18.45, pricedRatio: 0.21 },
        antigravity: { reqs: 391, nonCached: 4_175_179, cacheRead: 57_275_708, output: 769_124, reasoning: 0, costUsd: 0, pricedRatio: 0 },
      },
    },
    models: [
      { model: 'gpt-6.2-astra', provider: 'codex', reqs: 409, nonCached: 967_762, cacheRead: 19_378_560, output: 274_971, reasoning: 41_822, costUsd: null },
      { model: 'Gemini 4 Neon (High)', provider: 'antigravity', reqs: 128, nonCached: 1_420_500, cacheRead: 18_960_200, output: 248_900, reasoning: 0, costUsd: null },
      { model: 'gpt-5.4-mini', provider: 'codex', reqs: 24, nonCached: 312_000, cacheRead: 2_480_000, output: 46_200, reasoning: 8_120, costUsd: 0.63 },
      { model: 'gpt-6.2-sol', provider: 'codex', reqs: 7, nonCached: 35_676, cacheRead: 86_400, output: 2_906, reasoning: 802, costUsd: null },
    ],
    trend: [
      { label: '11-20', cache: 9.2, input: 0.8, output: 0.22 },
      { label: '11-21', cache: 14.5, input: 1.1, output: 0.34 },
      { label: '11-22', cache: 6.4, input: 0.5, output: 0.16 },
      { label: '11-23', cache: 18.8, input: 1.3, output: 0.41 },
      { label: '11-24', cache: 24.2, input: 1.6, output: 0.52 },
      { label: '11-25', cache: 28.6, input: 1.9, output: 0.58 },
      { label: '11-26', cache: 38.4, input: 2.4, output: 0.53 },
    ],
    heatmap: makeHeatmap([0, 1, 1, 0, 2, 1, 0, 2, 3, 2, 1, 0, 2, 2, 3, 1, 0, 1, 2, 3, 2, 2, 3, 2, 3, 4, 3, 3, 4, 4]),
  },
  expired: {
    coverage: 'Complete',
    latestTurnCache: 0.842,
    codexLatestTurnCache: 0.842,
    agLatestTurnCache: 0.915,
    quotas: [
      {
        provider: 'codex',
        label: 'Codex · 5h Window',
        period: '5h',
        kind: 'expired',
        lastUsedPct: 74,
        expiredText: '19:30',
        observedText: '18:55',
      },
      {
        provider: 'codex',
        label: 'Codex · Weekly (7d)',
        period: '7d',
        kind: 'active',
        usedPct: 32,
        resetsText: '4d 06h',
        observedText: '18:55',
      },
    ],
    credits: {
      planType: 'plus',
      serviceTier: 'standard',
      stateText: 'Limited · $12.4000',
      observedText: '18:55',
    },
    ranges: {
      today: {
        codex: { reqs: 52, nonCached: 248_000, cacheRead: 1_320_000, output: 44_200, reasoning: 9_100, costUsd: 1.48, pricedRatio: 1.0 },
        antigravity: { reqs: 18, nonCached: 180_000, cacheRead: 1_940_000, output: 29_500, reasoning: 0, costUsd: 0, pricedRatio: 0 },
      },
      '7d': {
        codex: { reqs: 310, nonCached: 1_120_000, cacheRead: 12_800_000, output: 210_000, reasoning: 41_000, costUsd: 8.95, pricedRatio: 0.94 },
        antigravity: { reqs: 95, nonCached: 860_000, cacheRead: 10_400_000, output: 140_000, reasoning: 0, costUsd: 0, pricedRatio: 0 },
      },
      '30d': {
        codex: { reqs: 740, nonCached: 2_650_000, cacheRead: 34_200_000, output: 510_000, reasoning: 96_000, costUsd: 21.3, pricedRatio: 0.92 },
        antigravity: { reqs: 190, nonCached: 1_700_000, cacheRead: 22_100_000, output: 290_000, reasoning: 0, costUsd: 0, pricedRatio: 0 },
      },
    },
    models: [
      { model: 'GPT-5.4', provider: 'codex', reqs: 52, nonCached: 248_000, cacheRead: 1_320_000, output: 44_200, reasoning: 9_100, costUsd: 1.48 },
      { model: 'Gemini 4 Neon (High)', provider: 'antigravity', reqs: 18, nonCached: 180_000, cacheRead: 1_940_000, output: 29_500, reasoning: 0, costUsd: null },
    ],
    trend: [
      { label: '11-20', cache: 3.1, input: 0.4, output: 0.08 },
      { label: '11-21', cache: 5.4, input: 0.6, output: 0.14 },
      { label: '11-22', cache: 4.8, input: 0.5, output: 0.12 },
      { label: '11-23', cache: 2.2, input: 0.3, output: 0.06 },
      { label: '11-24', cache: 6.0, input: 0.7, output: 0.18 },
      { label: '11-25', cache: 4.4, input: 0.4, output: 0.11 },
      { label: '11-26', cache: 3.2, input: 0.4, output: 0.07 },
    ],
    heatmap: makeHeatmap([0, 0, 1, 2, 1, 0, 1, 2, 1, 0, 0, 1, 2, 2, 1, 1, 0, 2, 3, 2, 1, 1, 2, 1, 2, 3, 2, 2, 1, 1]),
  },
  restricted: {
    coverage: 'Partial',
    latestTurnCache: 0.124,
    codexLatestTurnCache: 0.124,
    agLatestTurnCache: 0.891,
    quotas: [
      {
        provider: 'codex',
        label: 'Codex · 5h Window',
        period: '5h',
        kind: 'active',
        usedPct: 92,
        resetsText: '0h 28m',
        observedText: '21:52',
      },
      {
        provider: 'antigravity',
        label: 'Claude Opus 5.5 Pool',
        period: '5h',
        kind: 'restricted',
        reason: 'Pro / Ultra tier required',
        observedText: '21:50',
      },
    ],
    credits: {
      planType: 'plus',
      serviceTier: 'fast',
      stateText: 'Limited · $0.8500',
      observedText: '21:52',
    },
    ranges: {
      today: {
        codex: { reqs: 194, nonCached: 3_420_000, cacheRead: 11_800_000, output: 312_000, reasoning: 64_000, costUsd: 16.2, pricedRatio: 0.64 },
        antigravity: { reqs: 42, nonCached: 510_000, cacheRead: 4_120_000, output: 88_000, reasoning: 0, costUsd: 0, pricedRatio: 0 },
      },
      '7d': {
        codex: { reqs: 880, nonCached: 9_100_000, cacheRead: 62_400_000, output: 940_000, reasoning: 185_000, costUsd: 48.5, pricedRatio: 0.72 },
        antigravity: { reqs: 210, nonCached: 2_300_000, cacheRead: 21_400_000, output: 360_000, reasoning: 0, costUsd: 0, pricedRatio: 0 },
      },
      '30d': {
        codex: { reqs: 1420, nonCached: 14_200_000, cacheRead: 112_000_000, output: 1_420_000, reasoning: 290_000, costUsd: 82.1, pricedRatio: 0.75 },
        antigravity: { reqs: 360, nonCached: 3_900_000, cacheRead: 38_500_000, output: 620_000, reasoning: 0, costUsd: 0, pricedRatio: 0 },
      },
    },
    models: [
      { model: 'GPT-5.6-SOL', provider: 'codex', reqs: 120, nonCached: 2_120_000, cacheRead: 8_400_000, output: 210_000, reasoning: 42_000, costUsd: 16.2 },
      { model: 'gpt-6.2-astra', provider: 'codex', reqs: 74, nonCached: 1_300_000, cacheRead: 3_400_000, output: 102_000, reasoning: 22_000, costUsd: null },
      { model: 'Gemini 4 Neon (High)', provider: 'antigravity', reqs: 42, nonCached: 510_000, cacheRead: 4_120_000, output: 88_000, reasoning: 0, costUsd: null },
    ],
    trend: [
      { label: '11-20', cache: 8.1, input: 1.5, output: 0.25 },
      { label: '11-21', cache: 11.4, input: 2.1, output: 0.34 },
      { label: '11-22', cache: 9.8, input: 1.8, output: 0.29 },
      { label: '11-23', cache: 15.2, input: 2.6, output: 0.45 },
      { label: '11-24', cache: 14.0, input: 2.4, output: 0.42 },
      { label: '11-25', cache: 17.5, input: 3.1, output: 0.51 },
      { label: '11-26', cache: 15.9, input: 3.9, output: 0.40 },
    ],
    heatmap: makeHeatmap([1, 2, 1, 2, 3, 2, 1, 2, 3, 3, 2, 2, 3, 4, 3, 2, 2, 3, 3, 4, 3, 3, 4, 3, 4, 4, 3, 4, 4, 4]),
  },
  empty: {
    coverage: 'Unknown',
    latestTurnCache: null,
    codexLatestTurnCache: null,
    agLatestTurnCache: null,
    quotas: [
      { provider: 'codex', label: 'Codex · 5h Window', period: '5h', kind: 'nodata' },
      { provider: 'codex', label: 'Codex · Weekly (7d)', period: '7d', kind: 'nodata' },
    ],
    credits: {
      planType: null,
      serviceTier: null,
      stateText: 'Unknown',
      observedText: '—',
    },
    ranges: {
      today: {
        codex: { reqs: 0, nonCached: 0, cacheRead: 0, output: 0, reasoning: 0, costUsd: 0, pricedRatio: null },
        antigravity: { reqs: 0, nonCached: 0, cacheRead: 0, output: 0, reasoning: 0, costUsd: 0, pricedRatio: null },
      },
      '7d': {
        codex: { reqs: 0, nonCached: 0, cacheRead: 0, output: 0, reasoning: 0, costUsd: 0, pricedRatio: null },
        antigravity: { reqs: 0, nonCached: 0, cacheRead: 0, output: 0, reasoning: 0, costUsd: 0, pricedRatio: null },
      },
      '30d': {
        codex: { reqs: 0, nonCached: 0, cacheRead: 0, output: 0, reasoning: 0, costUsd: 0, pricedRatio: null },
        antigravity: { reqs: 0, nonCached: 0, cacheRead: 0, output: 0, reasoning: 0, costUsd: 0, pricedRatio: null },
      },
    },
    models: [],
    trend: [
      { label: '11-20', cache: 0, input: 0, output: 0 },
      { label: '11-21', cache: 0, input: 0, output: 0 },
      { label: '11-22', cache: 0, input: 0, output: 0 },
      { label: '11-23', cache: 0, input: 0, output: 0 },
      { label: '11-24', cache: 0, input: 0, output: 0 },
      { label: '11-25', cache: 0, input: 0, output: 0 },
      { label: '11-26', cache: 0, input: 0, output: 0 },
    ],
    heatmap: makeHeatmap(new Array(30).fill(0)),
  },
};

const state = {
  theme: 'dark',
  lang: 'zh',
  scene: 'normal',
  provider: 'all',
  range: 'today',
  opacity: 92,
  clickThrough: false,
};

function t(key) {
  return I18N[state.lang][key] ?? key;
}

function fmtTokens(n) {
  if (n == null) return '—';
  if (n >= 1e9) return `${(n / 1e9).toFixed(2)}B`;
  if (n >= 1e6) return `${(n / 1e6).toFixed(2)}M`;
  if (n >= 1e3) return `${(n / 1e3).toFixed(1)}K`;
  return String(n);
}

function fmtPct(ratio) {
  if (ratio == null || !Number.isFinite(ratio)) return '—';
  return `${(ratio * 100).toFixed(1)}%`;
}

function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined) node.textContent = text;
  return node;
}

function svgEl(tag, attrs = {}) {
  const node = document.createElementNS('http://www.w3.org/2000/svg', tag);
  for (const [k, v] of Object.entries(attrs)) node.setAttribute(k, String(v));
  return node;
}

function sumRange(scene, rangeKey, providerFilter) {
  const r = scene.ranges[rangeKey];
  const parts = providerFilter === 'all' ? [r.codex, r.antigravity] : [r[providerFilter]];
  const agg = parts.reduce(
    (acc, p) => {
      acc.reqs += p.reqs;
      acc.nonCached += p.nonCached;
      acc.cacheRead += p.cacheRead;
      acc.output += p.output;
      acc.reasoning += p.reasoning;
      acc.costUsd += p.costUsd;
      return acc;
    },
    { reqs: 0, nonCached: 0, cacheRead: 0, output: 0, reasoning: 0, costUsd: 0 },
  );
  agg.totalInput = agg.nonCached + agg.cacheRead;
  agg.totalTokens = agg.totalInput + agg.output;
  agg.cacheHitRatio = agg.totalInput > 0 ? agg.cacheRead / agg.totalInput : null;
  const pricedSum = parts.reduce((acc, p) => acc + (p.pricedRatio ?? 0) * (p.nonCached + p.cacheRead + p.output), 0);
  agg.pricedRatio = agg.totalTokens > 0 ? pricedSum / agg.totalTokens : null;
  return agg;
}

function quotaColor(usedPct) {
  const rem = 100 - usedPct;
  if (rem <= 10) return 'var(--danger)';
  if (rem <= 25) return 'var(--warn)';
  return 'var(--ok)';
}

function showTip(evt, text) {
  const tip = document.getElementById('floating-tip');
  tip.textContent = text;
  tip.style.left = `${Math.min(window.innerWidth - 260, evt.clientX + 12)}px`;
  tip.style.top = `${Math.max(12, evt.clientY - 36)}px`;
  tip.classList.add('visible');
}

function hideTip() {
  document.getElementById('floating-tip').classList.remove('visible');
}

function renderCapsule(scene) {
  const ring = document.getElementById('capsule-ring');
  ring.replaceChildren();
  const primary = scene.quotas[0];
  const secondary = scene.quotas[1];
  const r1 = 11;
  const c1 = 2 * Math.PI * r1;
  const r2 = 7.2;
  const c2 = 2 * Math.PI * r2;

  ring.appendChild(svgEl('circle', { cx: 14, cy: 14, r: r1, fill: 'none', stroke: 'var(--track)', 'stroke-width': 2.8 }));
  ring.appendChild(svgEl('circle', { cx: 14, cy: 14, r: r2, fill: 'none', stroke: 'var(--track)', 'stroke-width': 2.3 }));

  if (primary?.kind === 'active') {
    const rem = Math.max(0, 100 - primary.usedPct) / 100;
    ring.appendChild(svgEl('circle', {
      cx: 14, cy: 14, r: r1, fill: 'none',
      stroke: quotaColor(primary.usedPct), 'stroke-width': 2.8,
      'stroke-linecap': 'round',
      'stroke-dasharray': `${(c1 * rem).toFixed(1)} ${c1.toFixed(1)}`,
      transform: 'rotate(-90 14 14)',
    }));
  } else if (primary?.kind === 'expired') {
    ring.appendChild(svgEl('circle', {
      cx: 14, cy: 14, r: r1, fill: 'none',
      stroke: 'var(--warn)', 'stroke-width': 2.4, 'stroke-dasharray': '3 3',
    }));
  }

  if (secondary?.kind === 'active') {
    const rem2 = Math.max(0, 100 - secondary.usedPct) / 100;
    ring.appendChild(svgEl('circle', {
      cx: 14, cy: 14, r: r2, fill: 'none',
      stroke: 'var(--accent-codex)', 'stroke-width': 2.3,
      'stroke-linecap': 'round',
      'stroke-dasharray': `${(c2 * rem2).toFixed(1)} ${c2.toFixed(1)}`,
      transform: 'rotate(-90 14 14)',
    }));
  }

  const todayAll = sumRange(scene, 'today', 'all');
  const capPrimary = document.getElementById('capsule-primary');
  const capSub = document.getElementById('capsule-sub');
  const capDot = document.getElementById('capsule-dot');

  if (primary?.kind === 'active') {
    capPrimary.textContent = `${100 - primary.usedPct}% ${t('remaining')}`;
    capSub.textContent = `${t('capToday')} ${fmtTokens(todayAll.totalTokens)} · ${t('latestTurn')} ${fmtPct(scene.latestTurnCache)}`;
  } else if (primary?.kind === 'expired') {
    capPrimary.textContent = state.lang === 'zh' ? '等待新活动' : 'Awaiting';
    capSub.textContent = `${t('capToday')} ${fmtTokens(todayAll.totalTokens)} · ${t('latestTurn')} ${fmtPct(scene.latestTurnCache)}`;
  } else {
    capPrimary.textContent = t('noData');
    capSub.textContent = `${t('capToday')} ${fmtTokens(todayAll.totalTokens)}`;
  }

  capDot.className = 'capsule-dot';
  if (primary?.kind === 'active' && 100 - primary.usedPct <= 10) capDot.classList.add('danger');
  else if (primary?.kind === 'expired' || (scene.latestTurnCache != null && scene.latestTurnCache < 0.25)) capDot.classList.add('warn');

  document.getElementById('overlay-capsule').classList.toggle('click-through', state.clickThrough);
  document.getElementById('overlay-card').classList.toggle('click-through', state.clickThrough);
}

function renderExpandedOverlay(scene) {
  const container = document.getElementById('ov-quotas');
  container.replaceChildren();

  for (const q of scene.quotas) {
    const row = el('div', 'ov-row');
    const lbl = el('span', 'ov-label', quotaLabel(q));
    const val = el('span', 'ov-val tnum');
    if (q.kind === 'active') {
      val.textContent = `${100 - q.usedPct}% ${t('remaining')} · ${q.resetsText}`;
      val.style.color = quotaColor(q.usedPct);
    } else if (q.kind === 'expired') {
      val.textContent = state.lang === 'zh' ? '已到期 · 等待新活动' : 'Expired · Awaiting';
      val.style.color = 'var(--warn)';
    } else if (q.kind === 'restricted') {
      val.textContent = state.lang === 'zh' ? '套餐受限' : 'Restricted';
      val.style.color = 'var(--text-secondary)';
    } else {
      val.textContent = t('noData');
      val.style.color = 'var(--text-muted)';
    }
    row.append(lbl, val);
    container.appendChild(row);

    const track = el('div', 'progress-track');
    const fill = el('div', 'progress-fill');
    if (q.kind === 'active') {
      fill.style.width = `${100 - q.usedPct}%`;
      fill.style.background = quotaColor(q.usedPct);
    } else if (q.kind === 'expired') {
      fill.style.width = `${Math.max(0, 100 - q.lastUsedPct)}%`;
      fill.style.background = 'var(--warn)';
      fill.style.opacity = '0.45';
    } else {
      fill.style.width = '0%';
    }
    track.appendChild(fill);
    container.appendChild(track);
  }

  const todayAll = sumRange(scene, 'today', 'all');
  const agToday = sumRange(scene, 'today', 'antigravity');
  document.getElementById('ov-today-tokens').textContent = fmtTokens(todayAll.totalTokens);
  document.getElementById('ov-cache-pair').textContent = `${fmtPct(scene.latestTurnCache)} / ${fmtPct(todayAll.cacheHitRatio)}`;
  document.getElementById('ov-ag-tokens').textContent = agToday.totalTokens > 0
    ? `${fmtTokens(agToday.totalTokens)} · ${fmtPct(scene.agLatestTurnCache)}`
    : t('noData');

  const statusBadge = document.getElementById('ov-status-badge');
  statusBadge.className = state.clickThrough ? 'badge badge-warn' : 'badge badge-ok';
  statusBadge.textContent = state.clickThrough ? t('ovPass') : t('codexOnlineTag');
}

function renderTrendChart(scene) {
  const host = document.getElementById('trend-chart');
  host.replaceChildren();
  const data = scene.trend;
  const W = 540;
  const H = 186;
  const pad = { top: 14, right: 12, bottom: 26, left: 42 };
  const plotW = W - pad.left - pad.right;
  const plotH = H - pad.top - pad.bottom;

  const totals = data.map(d => d.cache + d.input + d.output);
  const maxVal = Math.max(1, ...totals) * 1.15;

  const svg = svgEl('svg', { viewBox: `0 0 ${W} ${H}`, width: '100%', height: '100%' });

  for (let i = 0; i <= 3; i++) {
    const y = pad.top + (plotH * i) / 3;
    const val = maxVal * (1 - i / 3);
    svg.appendChild(svgEl('line', {
      x1: pad.left, x2: W - pad.right, y1: y, y2: y,
      stroke: 'var(--border-subtle)', 'stroke-dasharray': i === 3 ? '' : '3 3',
    }));
    const lbl = svgEl('text', {
      x: pad.left - 8, y: y + 4, 'text-anchor': 'end',
      fill: 'var(--text-muted)', 'font-size': 10.5, 'font-family': 'var(--font-mono)',
    });
    lbl.textContent = `${val.toFixed(val >= 10 ? 0 : 1)}M`;
    svg.appendChild(lbl);
  }

  const slotW = plotW / data.length;
  const barW = Math.min(34, slotW * 0.56);

  data.forEach((d, i) => {
    const x = pad.left + i * slotW + (slotW - barW) / 2;
    let curY = pad.top + plotH;
    const segs = [
      { v: d.cache, fill: 'var(--series-cache)' },
      { v: d.input, fill: 'var(--series-input)' },
      { v: d.output, fill: 'var(--series-output)' },
    ];
    for (const s of segs) {
      if (s.v <= 0) continue;
      const h = (s.v / maxVal) * plotH;
      curY -= h;
      const rect = svgEl('rect', {
        x, y: curY, width: barW, height: Math.max(1.5, h), rx: 2.5, fill: s.fill,
      });
      svg.appendChild(rect);
    }

    const hit = svgEl('rect', {
      x: pad.left + i * slotW, y: pad.top, width: slotW, height: plotH,
      fill: 'transparent', style: 'cursor:pointer',
    });
    const tipText = `${d.label}  |  ${t('seriesCache')} ${d.cache.toFixed(1)}M · ${t('seriesInput')} ${d.input.toFixed(1)}M · ${t('seriesOutput')} ${d.output.toFixed(2)}M`;
    hit.addEventListener('mousemove', e => showTip(e, tipText));
    hit.addEventListener('mouseleave', hideTip);
    svg.appendChild(hit);

    const xl = svgEl('text', {
      x: pad.left + i * slotW + slotW / 2, y: H - 7, 'text-anchor': 'middle',
      fill: 'var(--text-muted)', 'font-size': 10.5, 'font-family': 'var(--font-mono)',
    });
    xl.textContent = d.label;
    svg.appendChild(xl);
  });

  host.appendChild(svg);
}

function renderHeatmap(scene) {
  const grid = document.getElementById('heatmap-grid');
  grid.replaceChildren();
  for (const cell of scene.heatmap) {
    const box = el('div', `hm-cell hm-lv-${cell.level}`);
    const tip = `${cell.date}  |  ${fmtTokens(cell.tokens)} tokens · ${cell.reqs} reqs (Lv.${cell.level})`;
    box.addEventListener('mousemove', e => showTip(e, tip));
    box.addEventListener('mouseleave', hideTip);
    grid.appendChild(box);
  }
}

function renderQuotasPanel(scene) {
  const stack = document.getElementById('detail-quota-stack');
  stack.replaceChildren();

  for (const q of scene.quotas) {
    const item = el('div', 'quota-item');
    const top = el('div', 'ov-row');
    const left = el('div', 'ov-label');
    const pBadge = el('span', q.provider === 'codex' ? 'badge badge-codex' : 'badge badge-ag', q.provider === 'codex' ? 'Codex' : 'Antigravity');
    left.append(pBadge, el('span', '', quotaLabel(q)));
    const right = el('span', 'ov-val tnum');

    if (q.kind === 'active') {
      right.textContent = `${100 - q.usedPct}% ${t('remaining')}`;
      right.style.color = quotaColor(q.usedPct);
    } else if (q.kind === 'expired') {
      right.textContent = state.lang === 'zh' ? '等待新活动' : 'Awaiting';
      right.style.color = 'var(--warn)';
    } else if (q.kind === 'restricted') {
      right.textContent = state.lang === 'zh' ? '套餐受限' : 'Restricted';
      right.style.color = 'var(--text-secondary)';
    } else {
      right.textContent = t('noData');
      right.style.color = 'var(--text-muted)';
    }
    top.append(left, right);
    item.appendChild(top);

    const track = el('div', 'progress-track');
    const fill = el('div', 'progress-fill');
    if (q.kind === 'active') {
      fill.style.width = `${100 - q.usedPct}%`;
      fill.style.background = quotaColor(q.usedPct);
    } else if (q.kind === 'expired') {
      fill.style.width = `${Math.max(0, 100 - q.lastUsedPct)}%`;
      fill.style.background = 'var(--warn)';
      fill.style.opacity = '0.45';
    } else {
      fill.style.width = '0%';
    }
    track.appendChild(fill);
    item.appendChild(track);

    const meta = el('div', 'quota-meta tnum');
    if (q.kind === 'active') {
      meta.append(
        el('span', '', `${t('used')} ${q.usedPct}% · ${t('resetsIn')} ${q.resetsText}`),
        el('span', '', `${t('observedAt')} ${q.observedText}`),
      );
    } else if (q.kind === 'expired') {
      meta.append(el('span', '', `${t('awaitingReset')} (${q.expiredText})`));
    } else if (q.kind === 'restricted') {
      meta.append(el('span', '', `${t('planRestrictedPrefix')}${q.reason}`));
    } else {
      meta.append(el('span', '', t('noData')));
    }
    item.appendChild(meta);
    stack.appendChild(item);
  }

  // Credits card
  const cred = el('div', 'quota-item');
  const cTop = el('div', 'ov-row');
  cTop.append(el('span', 'ov-label', t('creditsLabel')), el('span', 'ov-val mono', creditStateText(scene.credits.stateText)));
  const cMeta = el('div', 'quota-meta');
  const planTag = scene.credits.planType
    ? `plan: ${scene.credits.planType}${scene.credits.serviceTier ? ` · tier: ${scene.credits.serviceTier}` : ''}`
    : t('noData');
  cMeta.append(el('span', 'mono', planTag), el('span', '', t('accountUnconfirmed')));
  cred.append(cTop, cMeta);
  stack.appendChild(cred);

  const agNote = el('div', 'honest-note', t('agOfflineNotice'));
  stack.appendChild(agNote);
}

function renderModelsTable(scene) {
  const tbody = document.getElementById('model-tbody');
  tbody.replaceChildren();
  const rows = scene.models.filter(m => state.provider === 'all' || m.provider === state.provider);
  if (rows.length === 0) {
    const tr = el('tr');
    const td = el('td', 'empty-state', t('emptyModels'));
    td.colSpan = 8;
    tr.appendChild(td);
    tbody.appendChild(tr);
    return;
  }

  for (const m of rows) {
    const tr = el('tr');
    const tdModel = el('td');
    const wrap = el('div', 'model-cell');
    wrap.appendChild(el('span', '', m.model));
    tdModel.appendChild(wrap);

    const tdProv = el('td');
    tdProv.appendChild(el('span', m.provider === 'codex' ? 'badge badge-codex' : 'badge badge-ag', m.provider === 'codex' ? 'Codex' : 'Antigravity'));

    const totalIn = m.nonCached + m.cacheRead;
    const hit = totalIn > 0 ? m.cacheRead / totalIn : null;

    const tdHit = el('td', 'num tnum');
    const meter = el('div', 'mini-meter');
    const bar = el('div', 'mini-meter-bar');
    const fill = el('div', 'mini-meter-fill');
    fill.style.width = `${hit != null ? Math.round(hit * 100) : 0}%`;
    if (hit != null && hit < 0.35) fill.style.background = 'var(--warn)';
    bar.appendChild(fill);
    meter.append(bar, el('span', '', fmtPct(hit)));
    tdHit.appendChild(meter);

    const tdCost = el('td', 'num tnum');
    if (m.costUsd != null) {
      tdCost.textContent = `$${m.costUsd.toFixed(2)}`;
    } else {
      tdCost.appendChild(el('span', 'badge badge-neutral', t('unpricedLabel')));
    }

    const outText = m.reasoning > 0 ? `${fmtTokens(m.output)} (${fmtTokens(m.reasoning)})` : fmtTokens(m.output);
    tr.append(
      tdModel,
      tdProv,
      el('td', 'num tnum', String(m.reqs)),
      el('td', 'num tnum', fmtTokens(m.nonCached)),
      el('td', 'num tnum', fmtTokens(m.cacheRead)),
      el('td', 'num tnum', outText),
      tdHit,
      tdCost,
    );
    tbody.appendChild(tr);
  }
}

function renderAll() {
  document.documentElement.setAttribute('data-theme', state.theme);
  document.documentElement.style.setProperty('--overlay-opacity', (state.opacity / 100).toFixed(2));

  document.querySelectorAll('[data-i18n]').forEach(node => {
    node.textContent = t(node.getAttribute('data-i18n'));
  });

  const scene = SCENES[state.scene];
  const agg = sumRange(scene, state.range, state.provider);

  // KPIs
  document.getElementById('kpi-total-val').textContent = fmtTokens(agg.totalTokens);
  document.getElementById('kpi-total-sub').textContent = `${t('seriesInput')} ${fmtTokens(agg.nonCached)} · ${t('seriesOutput')} ${fmtTokens(agg.output)}`;

  const sBar = document.getElementById('kpi-stack-bar');
  sBar.replaceChildren();
  if (agg.totalTokens > 0) {
    const segs = [
      { v: agg.cacheRead, c: 'var(--series-cache)' },
      { v: agg.nonCached, c: 'var(--series-input)' },
      { v: Math.max(0, agg.output - agg.reasoning), c: 'var(--series-output)' },
      { v: agg.reasoning, c: 'var(--series-reason)' },
    ];
    for (const s of segs) {
      if (s.v <= 0) continue;
      const part = el('div');
      part.style.width = `${(s.v / agg.totalTokens) * 100}%`;
      part.style.background = s.c;
      sBar.appendChild(part);
    }
  }

  document.getElementById('kpi-cache-val').textContent = fmtPct(agg.cacheHitRatio);
  const latestRatio = state.provider === 'codex'
    ? scene.codexLatestTurnCache
    : state.provider === 'antigravity'
      ? scene.agLatestTurnCache
      : scene.latestTurnCache;
  const latestNode = document.getElementById('kpi-latest-turn-val');
  latestNode.textContent = fmtPct(latestRatio);
  latestNode.style.color = latestRatio == null ? 'var(--text-muted)' : latestRatio < 0.35 ? 'var(--warn)' : 'var(--ok)';

  document.getElementById('kpi-cost-val').textContent = agg.pricedRatio && agg.pricedRatio > 0 ? `$${agg.costUsd.toFixed(2)}` : '—';
  document.getElementById('kpi-priced-ratio').textContent = agg.pricedRatio != null ? fmtPct(agg.pricedRatio) : '—';
  document.getElementById('kpi-priced-bar').style.width = `${agg.pricedRatio != null ? Math.round(agg.pricedRatio * 100) : 0}%`;

  document.getElementById('kpi-reqs-val').textContent = agg.reqs.toLocaleString();
  const covBadge = document.getElementById('kpi-coverage-badge');
  if (scene.coverage === 'Complete') {
    covBadge.className = 'badge badge-ok';
    covBadge.textContent = t('coverageComplete');
  } else if (scene.coverage === 'Partial') {
    covBadge.className = 'badge badge-warn';
    covBadge.textContent = t('coveragePartial');
  } else {
    covBadge.className = 'badge badge-neutral';
    covBadge.textContent = t('coverageUnknown');
  }

  renderCapsule(scene);
  renderExpandedOverlay(scene);
  renderTrendChart(scene);
  renderHeatmap(scene);
  renderQuotasPanel(scene);
  renderModelsTable(scene);
  syncActiveButtons();
}

function syncActiveButtons() {
  document.querySelectorAll('[data-bind]').forEach(btn => {
    const [key, val] = btn.getAttribute('data-bind').split(':');
    btn.classList.toggle('active', String(state[key]) === val);
  });
  document.getElementById('opacity-val').textContent = `${state.opacity}%`;
}

function bindEvents() {
  document.querySelectorAll('[data-bind]').forEach(btn => {
    btn.addEventListener('click', () => {
      const [key, val] = btn.getAttribute('data-bind').split(':');
      if (key === 'clickThrough') state.clickThrough = !state.clickThrough;
      else state[key] = val;
      renderAll();
    });
  });

  document.getElementById('opacity-slider').addEventListener('input', e => {
    state.opacity = Number(e.target.value);
    renderAll();
  });

  for (const [id, fmt] of [['btn-export-json', 'JSON'], ['btn-export-csv', 'CSV']]) {
    document.getElementById(id).addEventListener('click', () => {
      const status = document.getElementById('export-feedback');
      status.textContent = `${t('exportToast')}${fmt}`;
      setTimeout(() => { status.textContent = ''; }, 2600);
    });
  }
}

function quotaLabel(q) {
  if (state.lang === 'zh') {
    if (q.label === 'Codex · 5h Window') return 'Codex · 5 小时窗口';
    if (q.label === 'Codex · Weekly (7d)') return 'Codex · 每周窗口 (7d)';
    if (q.label === 'Claude Opus 5.5 Pool') return 'Claude Opus 5.5 共享池';
  }
  return q.label;
}

function creditStateText(raw) {
  if (state.lang === 'zh') {
    if (raw === 'None (Included Plan)') return '无（订阅包含）';
    if (raw === 'Unknown') return '未知';
    if (raw.startsWith('Limited · ')) return raw.replace('Limited · ', '有限余额 · ');
  }
  return raw;
}

function applyQueryParams() {
  const raw = window.location.search || (window.location.hash.startsWith('#') ? `?${window.location.hash.slice(1)}` : '');
  const params = new URLSearchParams(raw);
  if (params.get('theme') === 'light' || params.get('theme') === 'dark') state.theme = params.get('theme');
  if (params.get('lang') === 'en' || params.get('lang') === 'zh') state.lang = params.get('lang');
  if (SCENES[params.get('scene')]) state.scene = params.get('scene');
}

applyQueryParams();
bindEvents();
renderAll();
})();
