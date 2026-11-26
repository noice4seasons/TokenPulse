# 阶段一源码审计与实测结论摘要

> **上游基线**：`https://github.com/jeongwookie/WhereMyTokens` @ Commit `6cd99eb0b41d0ef2b306fb2553545159ac347819` (v1.26.0, 2026-11-06)  
> **审计完成日期**：2026-11-26  
> **审计方式**：Gemini 主控源码通读与本机真实数据实测 + Codex (`gpt-6.2-astra`, `xhigh`) 两轮独立深度审计

---

## 一、核心发现速览

1. **本机真实数据重大发现（超越原版）**：
   - 实测本机 `%USERPROFILE%\.codex\sessions\**\*.jsonl`（25 个文件 / 77.37 MB）证实：`event_msg/token_count` 的 `payload.rate_limits` **不仅包含 `primary` (5h) / `secondary` (7d) 窗口，而且已直接包含 `credits: { has_credits, unlimited, balance }` 和 `plan_type`**（上游代码漏读了离线 `credits`，且在云端适配器里也丢弃了 `balance`）。
   - 同时观测到 `token_usage_record` 与 `event_msg/token_count` 成对出现，验证了移植上游 `accounting.ts` v4 防复算状态机的必要性。
2. **Codex 路径最终裁定（选项 1：V1 彻底零外网）**：
   - 上游 `codexUsageFetcher.ts` 会读取 `auth.json` 并向非公开端点 `chatgpt.com/backend-api/wham/usage` 发请求。
   - 结合本机离线实测，Codex 经独立评估正式裁定：**V1 Codex 路径彻底零外网、绝不接触 `auth.json`**，全部统计与最近观测额度/Credits 均来自本地只读 JSONL。
3. **Antigravity 本地 RPC 安全与实现发现**：
   - 上游通过反复拉起 `powershell.exe` 枚举 `language_server*.exe` 命令行获取 `--csrf_token` 与端口，再通过 `127.0.0.1` Connect-RPC 调用 `RetrieveUserQuotaSummary`（动态共享额度组）和 `GetUserStatus`（旧版按模型降级）。
   - 发现上游风险：PowerShell 高开销、端口未做 PID 归属交叉校验、允许 HTTP 降级与 `rejectUnauthorized: false`、探测响应无体积上限。
   - 结论：TokenPulse 改用原生 Win32 非提权探针核验进程与端口归属，阶段二先验证本地 TLS 信任材料可行性，严禁调用含对话正文的完整 `GetCascadeTrajectory`。
4. **许可证与品牌合规**：
   - 上游 `LICENSE` 为 MIT License，允许源码移植；但 `EULA.txt` 第 4/7 节禁止衍生品以引起官方来源混淆的方式使用 `"WhereMyTokens"` 名称及原作者图标。
   - 结论：启用全新独立名称 **`TokenPulse`**、全原创图标与新建独立 GitHub 仓库，并在 [THIRD_PARTY_NOTICES.md](../THIRD_PARTY_NOTICES.md) 中完整保留上游 MIT 版权声明。
5. **上游必须修复的核心工程缺陷**：
   - `sqliteUsageIndexStorage.ts` 的 `rebuild` 路径先按指纹跳过未变更条目，再按范围删除旧条目并扣减聚合桶，会导致未变化历史丢失 → TokenPulse 改为“完整覆盖证明 + 四态原子提交”。
   - `fileCheckpoint.ts` 每次增量校验从偏移量 0 全前缀重读算 SHA-256，大文件 I/O 浪费严重 → TokenPulse 改为“已提交前缀内 O(1) 头尾窗口校验（支持 `< 4 KiB` 短文件）+ 后台周期完整复核”。
   - `codex/usageIndexScanner.ts` 缺失 `used_percent` 时误填 `0%`，`codexAccount.ts` 未遵循 `CODEX_HOME` → 均在 TokenPulse 中彻底修复。

---

## 二、已拍板架构与跨电脑开发衔接清单

- **架构路线**：单进程 Rust 核心 + Win32 原生托盘 + Direct2D/DirectWrite/DirectComposition 矢量悬浮窗 + 按需 WebView2 详情面板（关闭即销毁专属浏览器进程）。
- **前端分工**：桌面悬浮窗高保真设计稿、详细统计面板纯原生前端（HTML/CSS/ESM/SVG，零 React/Node 重依赖）及全部图标均由 **Gemini 全权设计**。
- **另一台电脑进入阶段二的准备**：
  1. 克隆本仓库：`git clone https://github.com/noice4seasons/TokenPulse.git`
  2. 安装开发工具链（若目标机尚未安装）：
     - Rust (`rustup`)：`winget install Rustlang.Rustup`
     - Visual Studio C++ Build Tools (MSVC linker & Windows SDK)：`winget install Microsoft.VisualStudio.2022.BuildTools`
  3. 确认已读冻结方案 [docs/DEVELOPMENT_PLAN.md](DEVELOPMENT_PLAN.md)，按第七节直接启动**阶段二**（三大前置小探针 + `core-domain` / `provider-codex` / `store-sqlite` / `scheduler`）。
