(() => {
  'use strict';

  const PROTOCOL_VERSION = 1;

  const I18N = {
    ZhCn: {
      privacyOffline: '纯离线只读 · 零外网 · 零凭据',
      filterAll: '全部平台',
      rangeToday: '今日',
      range7d: '近 7 天',
      range30d: '近 30 天',
      btnRefresh: '刷新',
      exportJson: '导出 JSON',
      exportCsv: '导出 CSV',
      btnSettings: '设置',
      kpiTotal: '总 Token 消耗',
      kpiCache: '双维缓存命中率',
      kpiCost: 'API 等效估算',
      kpiReqs: '有效执行与覆盖度',
      latestTurn: '最近一轮',
      cumulative: '累计均值',
      notActualBill: '非订阅实付',
      pricedCoverage: '已定价覆盖率',
      trendTitle: '日 Token 消耗趋势',
      heatmapTitle: '近 30 天活跃热力图（5 档）',
      less: '低',
      more: '高',
      seriesCache: '缓存读取',
      seriesInput: '普通输入',
      seriesOutput: '输出',
      quotasTitle: '平台额度与 Credits 状态',
      accountUnconfirmed: '最近会话观测（账号未确认）',
      agOfflineNotice: 'Antigravity 纯离线日志统计已就绪（只统计本地 Token，零凭据/零正文；本地日志不含官方剩余额度百分比）。',
      agDisabledNotice: 'Antigravity 离线日志采集已关闭，可在右上角「设置」中开启。',
      awaitingReset: '已到预计重置时间，等待新活动（绝不自动补满 100%）',
      planRestrictedPrefix: '当前套餐无权限：',
      noData: '暂无数据',
      creditsLabel: 'Credits / 套餐',
      quota5h: '5小时窗口 (5h)',
      quota7d: '7天周额度 (7d)',
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
      coveragePartial: '覆盖不足',
      coverageUnknown: '等待扫描',
      emptyModels: '当前筛选范围暂无记录。',
      footerLeft: 'TokenPulse · 独立非官方开源实现，与 OpenAI、Google 无关联',
      footerRight: '只读本地日志 · 严禁触碰 auth.json 与对话正文 · 零额外模型 Token',
      settingsTitle: '偏好设置',
      themeLabel: '界面主题',
      themeDark: '黑曜石',
      themeLight: '青瓷霜',
      langLabel: '显示语言',
      placementLabel: '展示形态',
      placeTaskbar: '任务栏',
      placeDesktop: '桌面组件',
      placeFloating: '悬浮窗',
      opacityLabel: '不透明度',
      cfgCodexEnabled: '监控 Codex 离线日志',
      cfgAgEnabled: '监控 Antigravity 离线日志',
      cfgAlwaysOnTop: '悬浮窗始终置顶',
      cfgClickThrough: '悬浮窗点击穿透（仅悬浮窗生效）',
      cfgStartOnLogin: 'Windows 开机自启动',
      cfgLowThreshold: '低额度告警阈值',
      privacyBoundaryNote: '安全边界：本应用零外网请求、零遥测，不读取 auth.json 或对话正文；所有导出均由原生侧脱敏写盘。',
      remaining: '剩余',
      used: '已用',
      resetsIn: '重置倒计时',
      observedAt: '观测于',
      codexOnlineTag: '纯离线 JSONL',
      agOnlineTag: '纯离线 Transcript',
      updatedPrefix: '更新于 ',
    },
    EnUs: {
      privacyOffline: '100% Local Read-Only · Zero Network',
      filterAll: 'All Providers',
      rangeToday: 'Today',
      range7d: 'Last 7d',
      range30d: 'Last 30d',
      btnRefresh: 'Refresh',
      exportJson: 'Export JSON',
      exportCsv: 'Export CSV',
      btnSettings: 'Settings',
      kpiTotal: 'Total Tokens',
      kpiCache: 'Dual Cache Hit Rate',
      kpiCost: 'API-Equivalent Est.',
      kpiReqs: 'Requests & Coverage',
      latestTurn: 'Latest Turn',
      cumulative: 'Cumulative',
      notActualBill: 'Not subscription bill',
      pricedCoverage: 'Priced coverage',
      trendTitle: 'Daily Token Trend',
      heatmapTitle: '30-Day Activity Heatmap (5 Tiers)',
      less: 'Less',
      more: 'More',
      seriesCache: 'Cache Read',
      seriesInput: 'Uncached In',
      seriesOutput: 'Output',
      quotasTitle: 'Quotas & Credits Status',
      accountUnconfirmed: 'Recent local session (account unconfirmed)',
      agOfflineNotice: 'Antigravity offline transcript tracking is active (tokens only, zero credentials/body; local logs do not include official quota %).',
      agDisabledNotice: 'Antigravity offline tracking is disabled. Enable it in Settings.',
      awaitingReset: 'Reset time reached — awaiting new activity (never auto-filled to 100%)',
      planRestrictedPrefix: 'Plan restricted: ',
      noData: 'No Data',
      creditsLabel: 'Credits / Plan',
      quota5h: '5h Window',
      quota7d: '7d Weekly',
      tableTitle: 'Model Distribution',
      colModel: 'Model',
      colProvider: 'Provider',
      colReqs: 'Reqs',
      colInput: 'Uncached In',
      colCache: 'Cache Read',
      colOutput: 'Output (Reas.)',
      colCacheRate: 'Cache Hit',
      colCost: 'API Est.',
      unpricedLabel: 'Unpriced',
      coverageComplete: 'Complete',
      coveragePartial: 'Partial',
      coverageUnknown: 'Pending',
      emptyModels: 'No records in the selected window.',
      footerLeft: 'TokenPulse · Independent unofficial open-source project; not affiliated with OpenAI or Google',
      footerRight: 'Read-only local logs · Never reads auth.json or prompt bodies · 0 extra model tokens',
      settingsTitle: 'Preferences',
      themeLabel: 'Theme',
      themeDark: 'Obsidian',
      themeLight: 'Porcelain',
      langLabel: 'Language',
      placementLabel: 'Display Mode',
      placeTaskbar: 'Taskbar',
      placeDesktop: 'Desktop',
      placeFloating: 'Floating',
      opacityLabel: 'Opacity',
      cfgCodexEnabled: 'Track Codex offline logs',
      cfgAgEnabled: 'Track Antigravity offline logs',
      cfgAlwaysOnTop: 'Always on top (floating)',
      cfgClickThrough: 'Click-through (floating only)',
      cfgStartOnLogin: 'Start on Windows login',
      cfgLowThreshold: 'Low-quota alert threshold',
      privacyBoundaryNote: 'Security boundary: zero outbound network, zero telemetry, never reads auth.json or prompt bodies. Exports stay in native Rust.',
      remaining: 'left',
      used: 'used',
      resetsIn: 'Resets in',
      observedAt: 'Observed',
      codexOnlineTag: 'Offline JSONL',
      agOnlineTag: 'Offline Transcript',
      updatedPrefix: 'Updated ',
    },
  };

  const uiState = {
    provider: 'all',
    range: 'today',
    config: {
      theme: 'Dark',
      language: 'ZhCn',
      display_placement: 'Taskbar',
      overlay_opacity_pct: 90,
      always_on_top: true,
      click_through: false,
      start_on_login: false,
      low_quota_threshold_pct: 10,
      cooldown_minutes: 60,
      codex: { enabled: true },
      antigravity: { enabled: true, poll_interval_secs: 90 },
    },
    snapshot: null,
  };

  function t(key) {
    const lang = uiState.config.language === 'EnUs' ? 'EnUs' : 'ZhCn';
    return I18N[lang][key] ?? key;
  }

  function postToHost(command, payload = null) {
    if (window.chrome && window.chrome.webview && typeof window.chrome.webview.postMessage === 'function') {
      window.chrome.webview.postMessage({
        protocol_version: PROTOCOL_VERSION,
        command,
        payload,
      });
    }
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

  function fmtTokens(n) {
    if (n == null || !Number.isFinite(n)) return '—';
    if (n >= 1e9) return `${(n / 1e9).toFixed(2)}B`;
    if (n >= 1e6) return `${(n / 1e6).toFixed(2)}M`;
    if (n >= 1e3) return `${(n / 1e3).toFixed(1)}K`;
    return String(n);
  }

  function fmtPct(ratio) {
    if (ratio == null || !Number.isFinite(ratio)) return '—';
    return `${(ratio * 100).toFixed(1)}%`;
  }

  function fmtDurationMs(ms) {
    if (ms == null || ms <= 0) return uiState.config.language === 'EnUs' ? '0m' : '0分钟';
    const zh = uiState.config.language !== 'EnUs';
    if (ms > 86_400_000) {
      const totalHours = Math.ceil(ms / 3_600_000);
      const days = Math.floor(totalHours / 24);
      const hours = totalHours % 24;
      if (zh) return hours > 0 ? `${days}天${hours}时` : `${days}天`;
      return hours > 0 ? `${days}d ${hours}h` : `${days}d`;
    }
    const totalMin = Math.ceil(ms / 60_000);
    if (totalMin >= 1440) return zh ? '1天' : '1d';
    const hours = Math.floor(totalMin / 60);
    const mins = totalMin % 60;
    if (hours > 0) {
      if (zh) return mins > 0 ? `${hours}时${mins}分` : `${hours}小时`;
      return mins > 0 ? `${hours}h ${mins}m` : `${hours}h`;
    }
    return zh ? `${totalMin}分钟` : `${totalMin}m`;
  }

  function fmtResetTarget(resetsInMs, updatedAtMs) {
    if (resetsInMs == null || resetsInMs <= 0) return '';
    const dur = fmtDurationMs(resetsInMs);
    const base = updatedAtMs || Date.now();
    const target = new Date(base + resetsInMs);
    const now = new Date(base);
    const hh = String(target.getHours()).padStart(2, '0');
    const mm = String(target.getMinutes()).padStart(2, '0');
    const sameDay = target.getFullYear() === now.getFullYear()
      && target.getMonth() === now.getMonth()
      && target.getDate() === now.getDate();
    const clock = sameDay
      ? `${hh}:${mm}`
      : `${String(target.getMonth() + 1).padStart(2, '0')}/${String(target.getDate()).padStart(2, '0')} ${hh}:${mm}`;
    return uiState.config.language === 'EnUs'
      ? `${t('resetsIn')} ${dur} (${clock})`
      : `${dur}后重置 (${clock})`;
  }

  function isProPlan(planType) {
    if (!planType || typeof planType !== 'string') return false;
    return planType.trim().toLowerCase().startsWith('pro');
  }

  function periodLabel(q) {
    if (q.period_kind === 'FiveHour') return t('quota5h');
    if (q.period_kind === 'Weekly') return t('quota7d');
    return String(q.target_label || '');
  }

  function fmtTime(ms) {
    if (!ms) return '—';
    const d = new Date(ms);
    return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
  }

  function fmtDateUtc(ms) {
    const d = new Date(ms);
    return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
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

  function showToast(msg) {
    const node = document.getElementById('status-toast');
    node.textContent = msg;
    setTimeout(() => {
      if (node.textContent === msg) node.textContent = '';
    }, 3000);
  }

  function quotaColor(usedPct) {
    const rem = 100 - usedPct;
    if (rem <= 10) return 'var(--danger)';
    if (rem <= 25) return 'var(--warn)';
    return 'var(--ok)';
  }

  function formatCreditState(state) {
    if (state === 'Unknown') return t('noData');
    if (state === 'None') return uiState.config.language === 'EnUs' ? 'None' : '无（订阅包含）';
    if (state === 'Unlimited') return uiState.config.language === 'EnUs' ? 'Unlimited' : '无限额';
    if (state && typeof state === 'object' && 'Limited' in state) {
      const bal = state.Limited?.balance_text;
      return bal ? `$${bal}` : t('noData');
    }
    return t('noData');
  }

  function renderHeatmapAndTrend(cells) {
    const grid = document.getElementById('heatmap-grid');
    grid.replaceChildren();
    const list = Array.isArray(cells) ? cells.slice(-30) : [];
    for (const c of list) {
      const lv = Math.max(0, Math.min(4, Number(c.level || 0)));
      const box = el('div', `hm-cell hm-lv-${lv}`);
      const tip = `${fmtDateUtc(c.date_utc_start_ms)} | ${fmtTokens(c.total_tokens)} tokens · ${c.request_count} reqs (Lv.${lv})`;
      box.addEventListener('mousemove', e => showTip(e, tip));
      box.addEventListener('mouseleave', hideTip);
      grid.appendChild(box);
    }

    // Trend chart uses last 7 days from heatmap cells
    const host = document.getElementById('trend-chart');
    host.replaceChildren();
    const recent = list.slice(-7);
    const W = 540;
    const H = 184;
    const pad = { top: 14, right: 12, bottom: 26, left: 48 };
    const plotW = W - pad.left - pad.right;
    const plotH = H - pad.top - pad.bottom;
    const maxVal = Math.max(1, ...recent.map(d => d.total_tokens || 0)) * 1.15;
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
      lbl.textContent = fmtTokens(Math.round(val));
      svg.appendChild(lbl);
    }

    if (recent.length > 0) {
      const slotW = plotW / recent.length;
      const barW = Math.min(34, slotW * 0.56);
      recent.forEach((d, i) => {
        const x = pad.left + i * slotW + (slotW - barW) / 2;
        const h = ((d.total_tokens || 0) / maxVal) * plotH;
        if (h > 0) {
          svg.appendChild(svgEl('rect', {
            x, y: pad.top + plotH - h, width: barW, height: Math.max(2, h),
            rx: 3, fill: 'var(--series-cache)',
          }));
        }
        const hit = svgEl('rect', {
          x: pad.left + i * slotW, y: pad.top, width: slotW, height: plotH,
          fill: 'transparent',
        });
        const label = fmtDateUtc(d.date_utc_start_ms).slice(5);
        hit.addEventListener('mousemove', e => showTip(e, `${label} | ${fmtTokens(d.total_tokens)} tokens · ${d.request_count} reqs`));
        hit.addEventListener('mouseleave', hideTip);
        svg.appendChild(hit);

        const xl = svgEl('text', {
          x: pad.left + i * slotW + slotW / 2, y: H - 7, 'text-anchor': 'middle',
          fill: 'var(--text-muted)', 'font-size': 10.5, 'font-family': 'var(--font-mono)',
        });
        xl.textContent = label;
        svg.appendChild(xl);
      });
    }
    host.appendChild(svg);
  }

  function renderQuotas(providers) {
    const stack = document.getElementById('detail-quota-stack');
    stack.replaceChildren();
    let renderedAny = false;

    for (const p of providers || []) {
      const pName = p.provider === 'codex' ? 'Codex' : 'Antigravity';
      const pClass = p.provider === 'codex' ? 'badge badge-codex' : 'badge badge-ag';
      const hide5h = p.provider === 'codex' && isProPlan(p.credits?.plan_type);

      for (const q of p.quotas || []) {
        if (hide5h && q.period_kind === 'FiveHour') continue;
        renderedAny = true;
        const item = el('div', 'quota-item');
        const top = el('div', 'ov-row');
        const left = el('div', 'ov-label');
        left.append(el('span', pClass, pName), el('span', '', periodLabel(q)));
        const right = el('span', 'ov-val tnum');
        const track = el('div', 'progress-track');
        const fill = el('div', 'progress-fill');
        const meta = el('div', 'quota-meta tnum');

        const st = q.state;
        if (st && typeof st === 'object' && 'Active' in st) {
          const used = Number(st.Active.used_pct || 0);
          const rem = Math.max(0, Math.min(100, Number(st.Active.remaining_pct ?? (100 - used))));
          right.textContent = `${rem.toFixed(0)}% ${t('remaining')}`;
          right.style.color = quotaColor(used);
          fill.style.width = `${rem}%`;
          fill.style.background = quotaColor(used);
          const resetTxt = fmtResetTarget(st.Active.resets_in_ms, uiState.snapshot?.updated_at_ms);
          meta.append(
            el('span', '', `${t('used')} ${used.toFixed(0)}%${resetTxt ? ` · ${resetTxt}` : ''}`),
            el('span', '', `${t('observedAt')} ${fmtTime(q.observed_at_ms)}`),
          );
        } else if (st && typeof st === 'object' && 'AwaitingNewActivity' in st) {
          right.textContent = uiState.config.language === 'EnUs' ? 'Awaiting' : '等待新活动';
          right.style.color = 'var(--warn)';
          const lastUsed = st.AwaitingNewActivity.last_used_pct;
          if (typeof lastUsed === 'number') {
            fill.style.width = `${Math.max(0, 100 - lastUsed)}%`;
            fill.style.background = 'var(--warn)';
            fill.style.opacity = '0.45';
          }
          meta.append(el('span', '', t('awaitingReset')));
        } else if (st && typeof st === 'object' && 'PlanRestricted' in st) {
          right.textContent = uiState.config.language === 'EnUs' ? 'Restricted' : '套餐受限';
          right.style.color = 'var(--text-secondary)';
          meta.append(el('span', '', `${t('planRestrictedPrefix')}${st.PlanRestricted.reason || ''}`));
        } else {
          right.textContent = t('noData');
          right.style.color = 'var(--text-muted)';
          meta.append(el('span', '', t('noData')));
        }

        top.append(left, right);
        track.appendChild(fill);
        item.append(top, track, meta);
        stack.appendChild(item);
      }

      if (p.credits) {
        renderedAny = true;
        const cred = el('div', 'quota-item');
        const cTop = el('div', 'ov-row');
        cTop.append(el('span', 'ov-label', `${pName} · ${t('creditsLabel')}`), el('span', 'ov-val mono', formatCreditState(p.credits.state)));
        const cMeta = el('div', 'quota-meta');
        const planParts = [];
        if (p.credits.plan_type) planParts.push(`plan: ${p.credits.plan_type}`);
        if (p.credits.service_tier) planParts.push(`tier: ${p.credits.service_tier}`);
        cMeta.append(
          el('span', 'mono', planParts.join(' · ') || t('noData')),
          el('span', '', p.credits.account_confirmed ? '' : t('accountUnconfirmed')),
        );
        cred.append(cTop, cMeta);
        stack.appendChild(cred);
      }
    }

    if (!renderedAny) {
      stack.appendChild(el('div', 'empty-state', t('noData')));
    }
    stack.appendChild(el('div', 'honest-note', uiState.config.antigravity?.enabled ? t('agOfflineNotice') : t('agDisabledNotice')));
  }

  function renderModels(models) {
    const tbody = document.getElementById('model-tbody');
    tbody.replaceChildren();
    const rows = (models || []).filter(m => uiState.provider === 'all' || m.provider === uiState.provider);
    document.getElementById('table-count').textContent = `${rows.length}`;
    if (rows.length === 0) {
      const tr = el('tr');
      const td = el('td', 'empty-state', t('emptyModels'));
      td.colSpan = 8;
      tr.appendChild(td);
      tbody.appendChild(tr);
      return;
    }
    for (const row of rows) {
      const m = row.metrics;
      const tr = el('tr');
      const tdModel = el('td', 'model-cell', String(row.display_model || 'Unknown'));
      const tdProv = el('td');
      tdProv.appendChild(el('span', row.provider === 'codex' ? 'badge badge-codex' : 'badge badge-ag', row.provider === 'codex' ? 'Codex' : 'Antigravity'));

      const totalIn = (m.non_cached_input_tokens || 0) + (m.cache_read_tokens || 0);
      const hit = totalIn > 0 ? m.cache_read_tokens / totalIn : null;
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
      if (m.priced_request_count > 0) {
        tdCost.textContent = `$${(m.cost_micro_usd / 1_000_000).toFixed(2)}`;
      } else {
        tdCost.appendChild(el('span', 'badge badge-neutral', t('unpricedLabel')));
      }

      const outTxt = m.reasoning_output_tokens > 0
        ? `${fmtTokens(m.output_tokens)} (${fmtTokens(m.reasoning_output_tokens)})`
        : fmtTokens(m.output_tokens);

      tr.append(
        tdModel,
        tdProv,
        el('td', 'num tnum', String(m.request_count)),
        el('td', 'num tnum', fmtTokens(m.non_cached_input_tokens)),
        el('td', 'num tnum', fmtTokens(m.cache_read_tokens)),
        el('td', 'num tnum', outTxt),
        tdHit,
        tdCost,
      );
      tbody.appendChild(tr);
    }
  }

  function render() {
    document.documentElement.setAttribute('data-theme', uiState.config.theme === 'Light' ? 'light' : 'dark');
    document.querySelectorAll('[data-i18n]').forEach(n => { n.textContent = t(n.getAttribute('data-i18n')); });

    document.querySelectorAll('[data-filter]').forEach(b => b.classList.toggle('active', b.getAttribute('data-filter') === uiState.provider));
    document.querySelectorAll('[data-range]').forEach(b => b.classList.toggle('active', b.getAttribute('data-range') === uiState.range));
    document.querySelectorAll('[data-cfg-theme]').forEach(b => b.classList.toggle('active', b.getAttribute('data-cfg-theme') === uiState.config.theme));
    document.querySelectorAll('[data-cfg-lang]').forEach(b => b.classList.toggle('active', b.getAttribute('data-cfg-lang') === uiState.config.language));
    document.querySelectorAll('[data-cfg-placement]').forEach(b => b.classList.toggle('active', b.getAttribute('data-cfg-placement') === (uiState.config.display_placement || 'Taskbar')));

    document.getElementById('cfg-opacity').value = String(uiState.config.overlay_opacity_pct);
    document.getElementById('cfg-opacity-val').textContent = `${uiState.config.overlay_opacity_pct}%`;
    document.getElementById('cfg-codex-enabled').checked = Boolean(uiState.config.codex?.enabled);
    document.getElementById('cfg-ag-enabled').checked = Boolean(uiState.config.antigravity?.enabled);
    document.getElementById('cfg-topmost').checked = Boolean(uiState.config.always_on_top);
    document.getElementById('cfg-clickthrough').checked = Boolean(uiState.config.click_through);
    document.getElementById('cfg-autostart').checked = Boolean(uiState.config.start_on_login);
    document.getElementById('cfg-threshold').value = String(uiState.config.low_quota_threshold_pct);
    document.getElementById('cfg-threshold-val').textContent = `${uiState.config.low_quota_threshold_pct}%`;

    const snap = uiState.snapshot;
    if (!snap) return;

    const rangeAgg = uiState.range === '7d' ? snap.last_7_days : uiState.range === '30d' ? snap.last_30_days : snap.today;
    const m = rangeAgg.metrics;
    const totalIn = (m.non_cached_input_tokens || 0) + (m.cache_read_tokens || 0);
    const cacheRatio = totalIn > 0 ? m.cache_read_tokens / totalIn : null;

    document.getElementById('kpi-total-val').textContent = fmtTokens(m.total_tokens);
    document.getElementById('kpi-total-sub').textContent = `${t('seriesInput')} ${fmtTokens(m.non_cached_input_tokens)} · ${t('seriesOutput')} ${fmtTokens(m.output_tokens)}`;

    const sBar = document.getElementById('kpi-stack-bar');
    sBar.replaceChildren();
    if (m.total_tokens > 0) {
      for (const [val, color] of [
        [m.cache_read_tokens, 'var(--series-cache)'],
        [m.non_cached_input_tokens, 'var(--series-input)'],
        [Math.max(0, m.output_tokens - m.reasoning_output_tokens), 'var(--series-output)'],
        [m.reasoning_output_tokens, 'var(--series-reason)'],
      ]) {
        if (val <= 0) continue;
        const part = el('div');
        part.style.width = `${(val / m.total_tokens) * 100}%`;
        part.style.background = color;
        sBar.appendChild(part);
      }
    }

    document.getElementById('kpi-cache-val').textContent = fmtPct(cacheRatio);
    const latestNode = document.getElementById('kpi-latest-turn-val');
    latestNode.textContent = fmtPct(snap.latest_turn_cache_hit_ratio);
    latestNode.style.color = snap.latest_turn_cache_hit_ratio == null
      ? 'var(--text-muted)'
      : snap.latest_turn_cache_hit_ratio < 0.35
        ? 'var(--warn)'
        : 'var(--ok)';

    const pricedRatio = m.total_tokens > 0 ? m.priced_tokens / m.total_tokens : null;
    document.getElementById('kpi-cost-val').textContent = m.priced_request_count > 0 ? `$${(m.cost_micro_usd / 1_000_000).toFixed(2)}` : '—';
    document.getElementById('kpi-priced-ratio').textContent = pricedRatio != null ? fmtPct(pricedRatio) : '—';
    document.getElementById('kpi-priced-bar').style.width = `${pricedRatio != null ? Math.round(pricedRatio * 100) : 0}%`;

    document.getElementById('kpi-reqs-val').textContent = Number(m.request_count || 0).toLocaleString();
    const covBadge = document.getElementById('kpi-coverage-badge');
    if (rangeAgg.coverage === 'Complete') {
      covBadge.className = 'badge badge-ok';
      covBadge.textContent = t('coverageComplete');
    } else if (rangeAgg.coverage === 'Partial') {
      covBadge.className = 'badge badge-warn';
      covBadge.textContent = t('coveragePartial');
    } else {
      covBadge.className = 'badge badge-neutral';
      covBadge.textContent = t('coverageUnknown');
    }

    document.getElementById('updated-at').textContent = `${t('updatedPrefix')}${fmtTime(snap.updated_at_ms)}`;
    renderHeatmapAndTrend(snap.daily_heatmap);
    renderQuotas(snap.providers);
    renderModels(snap.model_distribution);
  }

  function pushConfigUpdate() {
    postToHost('update_config', uiState.config);
    render();
  }

  function bindUi() {
    document.querySelectorAll('[data-filter]').forEach(btn => {
      btn.addEventListener('click', () => {
        uiState.provider = btn.getAttribute('data-filter');
        postToHost('filter', { provider: uiState.provider, range: uiState.range });
        render();
      });
    });

    document.querySelectorAll('[data-range]').forEach(btn => {
      btn.addEventListener('click', () => {
        uiState.range = btn.getAttribute('data-range');
        postToHost('filter', { provider: uiState.provider, range: uiState.range });
        render();
      });
    });

    document.getElementById('btn-refresh').addEventListener('click', () => postToHost('refresh'));
    document.getElementById('btn-export-json').addEventListener('click', () => postToHost('export_json'));
    document.getElementById('btn-export-csv').addEventListener('click', () => postToHost('export_csv'));

    const backdrop = document.getElementById('settings-backdrop');
    document.getElementById('btn-settings').addEventListener('click', () => backdrop.classList.remove('hidden'));
    document.getElementById('btn-close-settings').addEventListener('click', () => backdrop.classList.add('hidden'));
    backdrop.addEventListener('click', e => { if (e.target === backdrop) backdrop.classList.add('hidden'); });

    document.querySelectorAll('[data-cfg-theme]').forEach(btn => {
      btn.addEventListener('click', () => {
        uiState.config.theme = btn.getAttribute('data-cfg-theme');
        pushConfigUpdate();
      });
    });
    document.querySelectorAll('[data-cfg-lang]').forEach(btn => {
      btn.addEventListener('click', () => {
        uiState.config.language = btn.getAttribute('data-cfg-lang');
        pushConfigUpdate();
      });
    });
    document.querySelectorAll('[data-cfg-placement]').forEach(btn => {
      btn.addEventListener('click', () => {
        uiState.config.display_placement = btn.getAttribute('data-cfg-placement');
        pushConfigUpdate();
      });
    });
    document.getElementById('cfg-codex-enabled').addEventListener('change', e => {
      uiState.config.codex = { ...(uiState.config.codex || {}), enabled: e.target.checked };
      pushConfigUpdate();
    });
    document.getElementById('cfg-ag-enabled').addEventListener('change', e => {
      uiState.config.antigravity = { ...(uiState.config.antigravity || { poll_interval_secs: 90 }), enabled: e.target.checked };
      pushConfigUpdate();
    });
    document.getElementById('cfg-opacity').addEventListener('input', e => {
      uiState.config.overlay_opacity_pct = Number(e.target.value);
      pushConfigUpdate();
    });
    document.getElementById('cfg-topmost').addEventListener('change', e => {
      uiState.config.always_on_top = e.target.checked;
      pushConfigUpdate();
    });
    document.getElementById('cfg-clickthrough').addEventListener('change', e => {
      uiState.config.click_through = e.target.checked;
      pushConfigUpdate();
    });
    document.getElementById('cfg-autostart').addEventListener('change', e => {
      uiState.config.start_on_login = e.target.checked;
      pushConfigUpdate();
    });
    document.getElementById('cfg-threshold').addEventListener('input', e => {
      uiState.config.low_quota_threshold_pct = Number(e.target.value);
      pushConfigUpdate();
    });

    if (window.chrome && window.chrome.webview) {
      window.chrome.webview.addEventListener('message', event => {
        const msg = event.data;
        if (!msg || typeof msg !== 'object' || msg.protocol_version !== PROTOCOL_VERSION) return;
        if (msg.type === 'snapshot' && msg.snapshot) {
          uiState.snapshot = msg.snapshot;
          if (msg.config) uiState.config = { ...uiState.config, ...msg.config };
          render();
        } else if (msg.type === 'toast' && typeof msg.message === 'string') {
          showToast(msg.message);
        }
      });
    }
  }

  bindUi();
  render();
  postToHost('ready');
})();
