# TokenPulse

> **Ultra-lightweight, 100% offline Windows 11 native AI Token & Quota monitor for OpenAI Codex and Google Antigravity.**  
> 面向 Windows 11 x64 的极致轻量、纯离线零外网 AI 用量与额度桌面监控工具，原生 Rust 单文件（约 **2 MB**），现已发布 **`v1.0.0`** 稳定版。

---

## 核心特性

### 1. 极致轻量 × Windows 11 原生三形态
- **单二进制 Rust 原生架构**：`Win32 系统托盘 + Direct3D11 / Direct2D / DirectWrite / DirectComposition 高 DPI 矢量界面 + 按需 WebView2 详情统计面板`，绿色便携无需安装、无需管理员权限、无需 Node.js / Electron。
- **三种桌面形态一键切换**（右键组件/托盘图标，或在设置抽屉中切换）：
  - **任务栏模式（默认）**：原生贴靠 Windows 11 任务栏托盘左侧空白处，通过任务栏所有权层级（`GWLP_HWNDPARENT`）与系统前台钩子稳固置顶不闪烁，支持水平微调拖动、全屏游戏/视频自动隐藏，单击直达详情面板。
  - **桌面组件模式**：贴合桌面底层（位于所有普通应用窗口之下），完全不遮挡工作区。
  - **置顶悬浮窗模式**：经典胶囊态 / 展开卡片态自由切换，支持 `30%~100%` 平滑不透明度、边缘吸附、多屏负坐标记忆与整窗点击穿透。
- **零帧空闲与按需销毁**：事件驱动脏检查渲染，空闲期严格 **0 帧提交**；关闭详情面板即彻底销毁专属 WebView2 浏览器进程，常驻内存极低。

### 2. 清晰直观的额度与用量呈现
- **套餐自适应（Pro / Pro Lite 友好）**：自动识别 Codex `pro` / `prolite` 等仅含 7 天周额度的套餐，自动隐藏空置的 5h 限制，改为清爽的单环与紧凑卡片展示。
- **人性化重置倒计时 + 本地时刻**：告别纯分钟读数，自动分级显示为 `6天11时后 (12/03 17:14)`、`3时15分后 (14:30)` 等直观格式，并按小时/分钟边界精确唤醒。
- **双平台纯离线精准统计**：
  - **OpenAI Codex**：移植并增强 v4 防复算状态机，离线解析 `rate_limits`（5h / 7d）与 `credits` 四态，支持普通输入 / 缓存读取 / 输出（含推理）与 API 等效估算。
  - **Google Antigravity**：流式只读解析本地 `transcript.jsonl` 数值与模型字段，实时统计离线 Token 消耗与缓存命中率。
- **深度分析面板**：内置深色（黑曜石）/ 浅色（青瓷霜）双主题与中/英文切换，提供**最近一轮 vs 累计双维缓存命中率**、近 7 天消耗趋势、**近 30 天 5 档活跃热力图**、模型分布表及原生脱敏 JSON / CSV 导出。

### 3. 账号安全与零外网红线
- **100% 纯离线只读**：全工作区 `Cargo.lock` **0 个 HTTP / TLS / 网络 crate**，静态二进制零云端端点，零遥测，零额外模型 Token 消耗。
- **绝不触碰敏感凭据与正文**：不读取 `auth.json` / OAuth Token，不采集或落盘任何对话正文；缺失或已到期的额度状态诚实展示，绝不伪造费用或自动补满 `100%`。
  - *说明：Antigravity 的本地日志文件仅包含 Token 与模型统计，不包含官方剩余额度百分比；遵循零凭据与不盲信自签证书的安全边界，V1 仅统计其离线 Token 消耗。*

---

## 快速开始

### 方式一：下载便携版（推荐）
1. 前往右侧 **[Releases](https://github.com/noice4seasons/TokenPulse/releases)** 下载 `TokenPulse-win11-x64-portable.zip`。
2. 解压后直接双击 `tokenpulse.exe`（无需安装、无需管理员权限）。
3. 程序默认停靠在 **Windows 11 任务栏托盘左侧**，并在系统托盘显示脉冲环图标：
   - **左键单击**任务栏小条或托盘图标：打开完整详情统计面板（趋势图 / 热力图 / 模型分布 / 偏好设置）。
   - **右键单击**任务栏小条或托盘图标：切换 **任务栏 / 桌面组件 / 悬浮窗** 模式、切换深浅色主题、切换中英文、开关开机自启或退出。
   - **拖动**：任务栏模式下支持沿任务栏水平微调；桌面组件 / 悬浮窗模式下可自由拖拽并自动吸附边缘。

> 系统要求：Windows 11 x64。详情统计面板复用系统自带的 Microsoft Edge WebView2 Runtime；即使缺失 Runtime，系统托盘与任务栏/桌面组件仍可独立完整运行。

---

### 方式二：从源码构建与验证
环境要求：Windows 11 x64 + Rust (`rustup`, `x86_64-pc-windows-msvc`) + Visual Studio 2022 Build Tools。

```powershell
# 运行全工作区单元测试与安全拒绝测试
cargo test --workspace

# 运行隐私哨兵、零外网审计与本机双平台端到端核验
powershell -ExecutionPolicy Bypass -File tests/benchmarks/verify_release.ps1

# 构建 Windows 11 x64 便携版 ZIP 与 SHA-256 校验文件
powershell -ExecutionPolicy Bypass -File scripts/build-portable.ps1
```

诊断 CLI 参数：
- `tokenpulse.exe --verify-all`：对本机 Codex 与 Antigravity 真实日志执行三轮（冷扫 / `O(1)` 增量 / 全量重建）一致性与隐私自检。
- `tokenpulse.exe --bench-desktop-ms 3000 [--bench-tray-only]`：测量冷启动耗时、内存占用与渲染提交次数。

---

## 工作区结构

| Crate / 目录 | 职责 |
|---|---|
| `crates/core-domain` | 领域模型、定价规则、额度状态机、强校验 DTO 与配置定义 |
| `crates/provider-codex` | Codex JSONL 只读扫描器、v4 防复算状态机、离线 `rate_limits` / `credits` 解析与 `O(1)` 双轨断点 |
| `crates/provider-antigravity` | Antigravity `transcript.jsonl` 流式纯数值解析器（零正文、零凭据）与 `O(1)` 双轨断点 |
| `crates/store-sqlite` | 内嵌 SQLite WAL 单写队列、乐观版本锁、四态原子提交、热力图与脱敏导出 |
| `crates/scheduler` | `ReadDirectoryChangesW` 递归目录监听、防抖合并与锁屏/休眠代次取消 |
| `crates/ui-win32` | Win32 原生托盘、Direct3D11/Direct2D/DirectComposition 任务栏贴靠/桌面组件/悬浮窗引擎 |
| `crates/ui-detail-web` | 按需 WebView2 详情窗口宿主（严格虚拟 Origin、`connect-src 'none'` CSP、关闭即销毁） |
| `crates/app` | 桌面主控进程、配置迁移、单实例守卫与验证 CLI |
| `frontend/app` | 编译期内嵌的纯原生 HTML / CSS / JS / SVG 详情统计面板 |

---

## 核心文档

- 正式架构与开发蓝图：[docs/DEVELOPMENT_PLAN.md](docs/DEVELOPMENT_PLAN.md)
- 安全与隐私边界说明：[docs/security-and-privacy.md](docs/security-and-privacy.md)
- 双端视觉设计规范：[docs/design-tokens.json](docs/design-tokens.json)
- 实机探针与验收报告：[docs/PHASE2_PROBE_REPORT.md](docs/PHASE2_PROBE_REPORT.md)
- 开源致谢与第三方许可：[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)

## 开源许可与致谢

本项目采用 [MIT License](LICENSE) 开源。部分数据解析与设计思路参考自 MIT 许可开源项目 [WhereMyTokens](https://github.com/jeongwookie/WhereMyTokens)、[agy-context](https://github.com/ahmetdotsh/agy-context) 与 [antigravity-context-viewer](https://github.com/elviscjn-cjn/antigravity-context-viewer)，详见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。  
本项目为独立非官方开源实现，与上述项目及 OpenAI、Google 无官方关联。
