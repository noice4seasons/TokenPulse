# TokenPulse

面向 Windows 11 的极致轻量、安全合规的 AI 用量监控工具（首批聚焦 **OpenAI Codex** 与 **Google Antigravity**）。

> 当前状态：**阶段一调查与安全审计已完成，v2.1 正式开发方案已冻结**，准备进入阶段二核心引擎与前置探针开发。

## 核心特点

- **账号安全第一**：Codex V1 **100% 纯离线、零外网、绝不接触 `auth.json`**；Antigravity 仅采用非提权 Win32 校验 + 受控本机回环只读接口；零模型 Token 消耗、零遥测、零客户端文件修改。
- **兼顾极致轻量与现代美学**：单进程 **Rust + Win32 托盘 + Direct2D/DirectComposition 矢量悬浮窗 + 按需 WebView2 详情面板**（关闭即销毁 WebView2 进程，常驻托盘 `< 20 MB`、悬浮窗 `< 40 MB`、便携包 `< 10 MB`）。
- **数据严谨准确**：移植并增强 Codex v4 防复算状态机、离线解析 `rate_limits` 与 `credits`、O(1) 增量断点与 SQLite WAL 事务聚合，缺失/过期一律如实展示“暂无数据 / 等待新活动”，绝不伪造数值。

## 核心文档

- 正式开发与架构蓝图（v2.1 冻结版）：[docs/DEVELOPMENT_PLAN.md](docs/DEVELOPMENT_PLAN.md)
- 阶段一源码审计与本机实测摘要：[docs/PHASE1_AUDIT_SUMMARY.md](docs/PHASE1_AUDIT_SUMMARY.md)
- 开源致谢与第三方许可声明：[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)

## 另一台电脑继续开发（快速上手）

1. 克隆仓库：
   ```powershell
   git clone https://github.com/noice4seasons/TokenPulse.git
   cd TokenPulse
   ```
2. 确认 Windows 11 工具链（详情前端采用纯原生 HTML/CSS/ESM/SVG，无需 Node.js）：
   ```powershell
   winget install Rustlang.Rustup
   winget install Microsoft.VisualStudio.2022.BuildTools
   ```
3. 直接从 [docs/DEVELOPMENT_PLAN.md](docs/DEVELOPMENT_PLAN.md) **第七节·阶段二** 开始实施：
   - 三大前置探针（Antigravity 非提权/TLS、WebView2 专属进程回收、DirectComposition 悬浮窗）
   - Rust Workspace 核心数据层（`core-domain`、`provider-codex`、`store-sqlite`、`scheduler`）

## 开源许可与致谢

本项目采用 [MIT License](LICENSE) 开源。部分数据解析与防复算设计思路参考自 MIT 许可开源项目 [WhereMyTokens](https://github.com/jeongwookie/WhereMyTokens)，详见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。本项目为独立非官方重构实现，与原项目及 OpenAI、Google 无官方关联。
