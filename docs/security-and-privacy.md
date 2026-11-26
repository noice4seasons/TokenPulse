# TokenPulse 安全与隐私透明说明

> 本文档向用户透明披露 **TokenPulse** 在 Windows 11 上的全部本地文件访问范围、网络行为、存储内容、系统集成边界与卸载方式。

---

## 一、核心安全承诺（编译期与架构硬约束）

1. **零外网业务请求与零遥测**：
   - TokenPulse 的完整 Rust 依赖树（`Cargo.lock`）中**不包含任何 HTTP/HTTPS/WebSocket 客户端、异步网络运行时或遥测库**。
   - 不内置崩溃自动上报、广告、静默后台自动更新或第三方中转代理。
2. **绝不接触任何账号凭据**：
   - 严禁读取、`stat`、监听、散列、修改或备份 `%CODEX_HOME%\auth.json`、Google OAuth Token、浏览器 Cookie 或其他凭据文件。
   - 路径校验层与 `ReadDirectoryChangesW` 文件监听层均对 `auth.json`、`state.vscdb`、`hub_*.json` 等凭据/状态文件名实施硬拒绝。
3. **零额外模型 Token 消耗**：
   - 绝不向任何模型发送提示词或探测请求，全生命周期额外推理 Token 恒为 `0`。
4. **绝不落盘用户对话正文**：
   - 对 Codex JSONL 与 Antigravity `transcript.jsonl` 均只做流式只读解析，仅抽取时间戳、模型标识、Token 数值维度、额度窗口与 Credits 状态；提示词、回答正文、思考过程与工具调用正文**绝不写入 SQLite 数据库、日志、导出文件或 WebView2 目录**。
5. **严格只读，绝不修改客户端文件**：
   - 不修改 Codex 或 Antigravity 的任何可执行文件、ASAR 包、配置或注册表，不注入进程，不 Hook API。

---

## 二、本地访问与存储范围清单

| 路径 | 访问方式 | 用途 |
|---|---|---|
| `%CODEX_HOME%\sessions`、`archived_sessions`、`session-cleanup-archive`（默认 `%USERPROFILE%\.codex\...`） | **只读**（共享读打开，拒绝 UNC/远程卷/重解析点） | 增量采集 Codex Token、离线 `rate_limits` 与 `credits` |
| `%CODEX_HOME%\.codex-global-state.json` | **只读**（体积上限 8 MiB） | 仅读取 `electron-persisted-atom-state.default-service-tier` |
| `%USERPROFILE%\.gemini\antigravity\brain\<cid>\.system_generated\logs\transcript.jsonl` | **只读**（仅反序列化 7 个数值/模型字段，跳过正文） | 增量采集 Antigravity 离线 Token、模型分布与缓存命中率 |
| `%LOCALAPPDATA%\TokenPulse\usage.db`（WAL 模式） | 读写 | 持久化脱敏汇总指标、小时/天/月聚合桶、断点哈希与额度观测 |
| `%LOCALAPPDATA%\TokenPulse\config.json` | 读写 | 持久化主题、语言、悬浮窗位置/透明度/模式与通知阈值 |
| `%LOCALAPPDATA%\TokenPulse\webview2\` | 读写（仅打开详情页时活跃） | 本应用独立专属 WebView2 `UserDataFolder`，关闭详情页即退出专属进程 |
| `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` (`TokenPulse`) | 仅用户显式启用开机自启时写入 | 当前用户级自启动，无需管理员权限 |

---

## 三、关于 Antigravity 在线额度接口的透明说明

- 阶段二实机探针确认：当前 Antigravity 2.35.2 本地回环 HTTPS 端口仅提供内置自签证书（无法通过 Windows 信任链校验），且不再通过命令行暴露只读凭据。
- TokenPulse 严守「绝不盲信自签证书（`rejectUnauthorized: false`）、绝不降级明文 HTTP、绝不绕过隔离边界」原则，因此 V1 中 Antigravity **仅采用 100% 纯离线 `transcript.jsonl` 数值采集**（完整支持 Token 统计、模型分布、最近一轮与累计缓存命中率及 5 档活跃热力图），不发起任何本地 RPC 或外网请求。

---

## 四、WebView2 详情面板安全边界与第三方说明

- 详情面板采用**按需创建、关闭即销毁**机制，常驻托盘与 Direct2D 悬浮窗期间 WebView2 进程数为 `0`。
- 详情页面全部静态资源在编译期内嵌到主程序，仅映射至虚拟 Origin `https://app.tokenpulse.local/`，并强制注入严格安全头：
  `Content-Security-Policy: default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; connect-src 'none'; object-src 'none'; frame-ancestors 'none'`
- 全面拦截外部导航、新窗口、权限申请与路径遍历；所有动态文本（模型名、标签等）均按纯文本 `textContent` 渲染，杜绝 XSS。
- **第三方边界声明**：TokenPulse 自身不发起任何外网连接，但无法代运行中的第三方应用（如 Antigravity IDE 或 Windows 系统级 WebView2 运行时内部行为）作出全机零出站承诺；在严格离线环境下，可仅使用纯 Rust 托盘与 Direct2D 悬浮窗（不打开 WebView2 详情页）。

---

## 五、完全卸载

1. 在托盘菜单中关闭「开机自启动」并选择「退出 TokenPulse」。
2. 删除便携目录中的 `tokenpulse.exe` 与随附文件。
3. 如需清除全部本地统计缓存与配置，删除 `%LOCALAPPDATA%\TokenPulse\` 即可，系统无任何残留驱动、服务或全局注册表项。
