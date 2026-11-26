# TokenPulse

面向 Windows 11 x64 的极致轻量、安全合规的 AI 用量监控桌面工具（首批全面支持 **OpenAI Codex** 与 **Google Antigravity**）。

> 当前状态：**V1.0 全阶段核心开发、实机安全审计、隐私哨兵与便携打包构建已完成**。

---

## 核心特点

- **账号安全与零外网红线**：
  - **100% 纯离线只读**：全工作区 `Cargo.lock` 零 HTTP/TLS/网络 crate；Codex 路径绝不触碰 `auth.json`；Antigravity 路径直接流式只读解析 `transcript.jsonl` 数值与模型字段（严禁读取对话正文与任何凭据，不向本地自签 RPC 发送令牌）。
  - **零额外模型 Token 消耗、零遥测、零客户端文件修改**。
- **极致轻量 × 现代 Windows 11 美学**：
  - 单二进制 Rust 原生架构：**Win32 系统托盘 + Direct3D11/Direct2D/DirectWrite/DirectComposition 高 DPI 矢量悬浮窗 + 按需 WebView2 详情统计面板**。
  - **双形态悬浮窗**：紧凑环形胶囊态（`244×46 DIP`）与展开卡片态（`332×236 DIP`）自由切换，支持 `30%~100%` 平滑透明度、边缘吸附、多屏负坐标记忆、整窗点击穿透与托盘一键解锁，**空闲期严格 0 帧提交**。
  - **关闭即销毁 WebView2**：按需打开详情面板（纯原生 HTML/CSS/JS/SVG，编译期内嵌 `< 30 KB`，严格虚拟 Origin 与 `connect-src 'none'` CSP 防线），关闭窗口即等待专属 `UserDataFolder` 浏览器进程完全退出。
- **数据严谨准确**：
  - 移植并增强 Codex v4 防复算状态机、离线解析 `rate_limits`（5h / 7d）与 `credits` 四态；常规 `O(1)`（`≤ 8 KiB`）+ 后台周期 `O(n)` 全校双轨断点；SQLite WAL 单写队列 + 乐观版本锁 + 四态原子提交 + UTC 整桶/边缘明细精确聚合。
  - 支持**最近一轮 vs 累计双维缓存命中率**与**近 30 天 5 档活跃热力图**；缺失/过期一律诚实展示，绝不伪造费用或自动补满 `100%`。

---

## 快速使用（便携版 / 源码构建）

### 方式一：运行便携版
双击构建产物 `dist/TokenPulse-win11-x64-portable/tokenpulse.exe`（无需管理员权限）：
- 系统托盘出现 TokenPulse 原创脉冲环图标，桌面默认展示紧凑悬浮窗。
- 单击托盘图标或右键菜单打开「详细统计面板」、切换深色（Obsidian Slate）/ 浅色（Porcelain Frost）主题、切换中/英文、开关开机自启或导出脱敏 JSON/CSV 统计。

### 方式二：从源码构建与验证
环境要求：Windows 11 x64 + Rust (`rustup`, `x86_64-pc-windows-msvc`) + Visual Studio 2022 Build Tools（无需 Node.js）。

```powershell
# 运行全工作区单元测试与安全拒绝测试
cargo test --workspace

# 运行阶段四隐私哨兵与双平台实机端到端验收
powershell -ExecutionPolicy Bypass -File tests/benchmarks/verify_release.ps1

# 构建 Windows 11 x64 便携版 ZIP 与 SHA-256 校验文件
powershell -ExecutionPolicy Bypass -File scripts/build-portable.ps1
```

诊断 CLI 参数：
- `tokenpulse.exe --verify-all`：对本机 Codex 与 Antigravity 真实日志执行三轮（冷扫 / O(1) 增量 / 全量重建）一致性与隐私自检。
- `tokenpulse.exe --bench-desktop-ms 3000 [--bench-tray-only]`：测量真实冷启动时间、内存占用与悬浮窗零帧提交。

---

## 核心文档

- 正式开发与架构蓝图（v2.1 冻结 + v2.2 修订）：[docs/DEVELOPMENT_PLAN.md](docs/DEVELOPMENT_PLAN.md)
- 阶段一源码审计与本机实测摘要：[docs/PHASE1_AUDIT_SUMMARY.md](docs/PHASE1_AUDIT_SUMMARY.md)
- 阶段二前置探针与双平台实机验收报告：[docs/PHASE2_PROBE_REPORT.md](docs/PHASE2_PROBE_REPORT.md)
- 双端统一视觉规范（冻结版）：[docs/design-tokens.json](docs/design-tokens.json)
- 安全与隐私透明说明：[docs/security-and-privacy.md](docs/security-and-privacy.md)
- 开源致谢与第三方许可声明：[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)

## 开源许可与致谢

本项目采用 [MIT License](LICENSE) 开源。部分数据解析与设计思路参考自 MIT 许可开源项目 [WhereMyTokens](https://github.com/jeongwookie/WhereMyTokens)、[agy-context](https://github.com/ahmetdotsh/agy-context) 与 [antigravity-context-viewer](https://github.com/elviscjn-cjn/antigravity-context-viewer)，详见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)。本项目为独立非官方开源实现，与原项目及 OpenAI、Google 无官方关联。
