# 阶段二高风险前置探针实机验收报告与架构裁决

> **测试环境**：Windows 11 x64 Build `26300.9032`（26H2）｜Rust `1.100.0` (`x86_64-pc-windows-msvc`)｜MSVC `14.44.35207`｜Windows 11 SDK `10.0.26100.0`  
> **探针代码位置**：[tools/phase2-probes](file:///d:/软件开发/04_WhereMyTokens%20Lite/tools/phase2-probes/Cargo.toml)（Release 构建）  
> **测试日期**：2026-11-26

---

## 一、三大探针实机结果总览

| 探针 | 目标与核心风险 | 实机关键指标 | 结果与裁决 |
|---|---|---|---|
| **探针 A**：[probe_antigravity.rs](file:///d:/软件开发/04_WhereMyTokens%20Lite/tools/phase2-probes/src/bin/probe_antigravity.rs) | 非提权 Win32 进程/端口核验 + 本地 TLS 验证材料可行性 | 发现与核验耗时 **`5 ms`**；PID/Session/User SID/路径/`GetExtendedTcpTable` 监听归属全部跑通；本机 Antigravity 2.35.2 无 `--csrf_token` 命令行参数、无 Authenticode 签名（`TRUST_E_NOSIGNATURE`）、回环 HTTPS 端口使用内置自签证书（`OU=bundled on purpose, O=ENABLES HTTP2, CN=localhost`）且无外部独立证书文件 | **非提权核验通过；按方案第 8.1 节裁定 V1 将 Antigravity 置于安全 `TlsUntrusted / Unsupported` 不可用或缓存态**，绝不降级明文、绝不盲信自签，不阻塞 Codex V1 交付 |
| **探针 B**：[probe_webview2.rs](file:///d:/软件开发/04_WhereMyTokens%20Lite/tools/phase2-probes/src/bin/probe_webview2.rs) | 同进程 UI STA 专属 `UserDataFolder` WebView2 连续 10 轮开关回收、创建中代次取消、缺 Runtime 降级 | WebView2 Runtime `131.0.2903.86`；10 轮均触发 `BrowserProcessExited`（打开 **`279~289 ms`**，首轮 `883 ms`；关闭退出 **`162~193 ms`**）；宿主 `Private Bytes` 稳定在 **`2.14~2.25 MB`**（零单调泄露），`Working Set` **`14.20~14.32 MB`**；代次取消丢弃迟到回调并 `164 ms` 退出进程；UDF 目录无锁清空；缺 Runtime 返回明确 `0x80070002` 无崩溃 | **`passed: true`（满分通过）**。裁定正式版直接采用 **同进程 UI STA 宿主方案**，无需拆 `--detail-host` 子进程 |
| **探针 C**：[probe_overlay.rs](file:///d:/软件开发/04_WhereMyTokens%20Lite/tools/phase2-probes/src/bin/probe_overlay.rs) | Direct3D11 + Direct2D + DirectWrite + DirectComposition 悬浮窗合成、透明度、点击穿透、零帧空转与内存占用 | `driver_type: "hardware"`；首帧 **`107 ms`**；`opacity_supported: true`（DirectComposition EffectGroup）；`click_through_toggled: true`（`WS_EX_TRANSPARENT`）；3 秒空闲期 **`idle_commits = 0`**（总提交 `2` 帧 = 首帧 + 1 次模拟数据更新）；`Private Bytes` **`14.46 MB`**，`Working Set` **`34.17 MB`**（低于 `< 40 MB` 硬线） | **`passed: true`（满分通过）**。裁定正式版锁定 **Direct2D/DirectWrite + DirectComposition 主渲染路径** |

---

## 二、关键工程细节备忘（供阶段三 / 阶段四直接遵循）

1. **WebView2 创建中取消边界行为（探针 B 实测发现）**：
   - 若在 `CreateCoreWebView2Controller` 异步回调返回之前即 `DestroyWindow(hwnd)`，WebView2 不会启动浏览器进程，因而也不会触发 `BrowserProcessExited`。
   - 正式 `ui-detail-web` 实现中，代次取消应仅当异步回调实际收到 `Some(controller)` 并调用 `controller.Close()` 后，才等待该实例的 `BrowserProcessExited`；若回调返回错误或未创建控制器，则立即结束等待避免白等 5 秒超时。
2. **Antigravity 2.35.2 合规冻结决策（探针 A 实测发现）**：
   - 严禁绕过隔离边界探测当前宿主服务或读取其 stdin/凭据；严禁 `rejectUnauthorized: false` 或 HTTP 明文降级。
   - 在 `crates/core-domain` 中已冻结 `ProviderAvailability::{NotRunning, AccessDenied, Unsupported, TlsUntrusted, CachedOnly, Disabled}` 与 `OverlaySnapshotDto` / `DashboardSnapshotDto`（`protocol_version = 1`，`#[serde(deny_unknown_fields)]`），UI 将诚实清晰地展示状态与上次有效缓存观测时间。
3. **版本对齐冻结**：
   - 本工作区统一锁定 `windows = "=0.58.0"` 与 `webview2-com = "=0.33.0"`，在 Windows 11 + Rust `1.100.0` 下已实测零告警编译通过。

---

## 三、阶段二核心数据层与本机 103.92 MB 真实日志验收摘要

- **Workspace 单元测试与集成测试**：`cargo test --workspace` **51 / 51 全部通过（0 error, 0 warning）**：
  - `tokenpulse-core-domain`：13 / 13
  - `tokenpulse-provider-codex`：21 / 21
  - `tokenpulse-scheduler`：8 / 8（含 Windows `ReadDirectoryChangesW` 真实递归监听、`auth.json` 过滤与停止退出测试）
  - `tokenpulse-store-sqlite`：8 / 8（含两次 rebuild 幂等、中断重建保护、23h/25h 夏令时与 `+05:30` 半小时时区整桶+边缘明细对照、脱敏导出）
  - `tokenpulse`（`crates/app` 端到端集成）：1 / 1（含分叉/双视图/坏行/同卷移入 `archived_sessions` 归档改名）
- **本机真实 Codex 数据集端到端核验**（`tokenpulse.exe --verify-codex`，Release 构建）：
  - 数据规模：**35 个 `.jsonl` 文件 / 103.92 MB**（`103,919,287` 字节），涵盖 `sessions` 与 `archived_sessions`。
  - **第 1 轮（冷扫 + SQLite WAL 提交）**：耗时 **`2,424 ms`**，提取 `1,306` 条去重执行，总 Token **`106,856,780`**（缓存命中率 `97.32%`），离线检出最新 `Weekly` 额度（`used_pct: 5.0%`）及 `plan_type: "prolite"` / `credits: None`，`files_with_gaps: 0`。
  - **第 2 轮（O(1) 增量复扫）**：耗时 **`62 ms`**，`max_validation_bytes_read = 8,192` 字节（严格 `≤ 8 KiB`），`total_payload_bytes_read = 0`，`committed_entries = 0`，全量汇总指标与第 1 轮 **100% 精确相等**。
  - **第 3 轮（全量 Rebuild）**：耗时 **`2,070 ms`**，全量汇总指标与第 1 轮 **100% 精确相等**（无历史丢失、无重复计费）。
  - **体积与安全红线**：Release 单二进制（含内嵌 SQLite）体积 **`1.62 MB`**（目标 `< 10 MB`）；`Cargo.lock` 网络相关 crate 数量 **`0`**；导出 JSON 与核验报告隐私自检全部通过（零绝对路径、零 `auth.json`、零对话正文）。

---

## 四、v2.2 社区调研吸收与双平台离线端到端验收（Codex + Antigravity）

- **Workspace 全量单测与集成测试**：`cargo test --workspace` **64 / 64 全部通过（0 error, 0 warning）**（新增 `tokenpulse-provider-antigravity` 12 项单元测试与 `tokenpulse-store-sqlite` 最近一轮缓存命中率/5 档热力图单元测试）。
- **双平台纯离线实机验收**（`tokenpulse.exe --verify-all`，Release 构建，**`passed = true`**）：
  - **Codex**：`36` 文件、`1,335` 条去重执行、`108,121,470` Tokens；冷扫 `2,533 ms`，第 2 轮 O(1) **`63 ms`**（`max_validation_bytes_read = 8,192 B`），最近一轮缓存命中率 **`97.39%`**，30 天 5 档活跃热力图正常生成。
  - **Antigravity（100% 纯离线 `transcript.jsonl` 数值解析，零凭据/零自签 RPC/零正文落盘）**：`13` 文件、`391` 条去重执行、**`62,220,011` Tokens**（非缓存输入 `4,175,179` + 缓存读 `57,275,708` + 输出 `769,124`）；冷扫 **`135 ms`**，第 2 轮 O(1) **`11 ms`**（`max_validation_bytes_read = 8,192 B`），最近一轮缓存命中率 **`99.23%`**，三轮冷扫/增量/Rebuild 指标 **100% 精确相等**。
  - **体积与安全红线**：双平台 Release 单二进制体积 **`1.70 MB`**；`Cargo.lock` 网络相关 crate 数量保持 **`0`**。


