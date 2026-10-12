> 本页保留 af20f6a 交付时的历史声明。2026-10-10 独立审查已否定其完整开发/验收结论；不能据本文将占位流程或 NOT_RUN 记为通过。当前代码与实际结果以 [修复记录](review-fixes-2026-10-10.md) 为准。

## 2026-10-11 续修实现

主代理实际 Doctor 47 项通过、1 项子进程fixture单独由owner测试调用；完整CLI 1296项通过。Windows强制终止child的原socket可能报告ConnectionReset，实际receipt保持strict_peer_eof=false，不将其改成EOF。[本轮受测范围与日志](continuation-2026-10-11.md)优先于下方历史和编辑时NOT_RUN文字。

- macOS 以有界 `launchctl print system/com.apple.remoted` 读取真实 state/pid；保存观察时间。只有前后两次都观察到运行进程，且 existing RSD 目录确认 selected UDID，才报告本次有限共存观察 OK。两点采样不证明期间持续存活。未选择 RSD、缺 PID/状态、目录失败/错目标均保持 Unknown；不停止系统 remoted。
- Host subprocess 使用一个从 startup 前计时的绝对期限、1 MiB stdout 上限和独立 2 秒 kill/wait 收尾预算。Ctrl-C 请求传到进程 owner 后继续等待该 owner 收尾；原输出/IO/timeout/cancel 原因和次级 kill/wait 错误均保留。Windows child 使用 CREATE_NO_WINDOW。OS 的同步 spawn 系统调用本身不能物理中止；回来后立即检查原期限，不能用延迟成功越过时钟。
- 显式 selected UDID 下新增 fresh existing-trust `CopyDevices` 探针，严格区分缺 EntryList、空列表、Developer/Personalized DiskImageType 与 malformed。签名探针、挂载观察与本地镜像目录仍为各自证据，不互相推断。
- `ddi_usable` 在 legacy Instruments/secure Instruments 路径上实际执行只读 DeviceInfo `runningProcesses`，复用原 DTX 的 checked array/dictionary decoder；全流程共用原探针绝对期限、StartService 的实际 SSL 状态与现有 pair TLS。仅报告该具体 developer operation 可用，不声明 Apple 签名验证，也不扩大为所有 developer services。ProductVersion 只选择已实现 transport；它不能使探针自动通过。
- iOS 17+ 的 checked existing-RSD developer-operation adapter 仍缺实现，此项显式 Unknown/unsupported，不隐式建 tunnel/配对/下载/挂载 DDI。所有 physical 和 macOS launchctl 实测本机仍 NOT_RUN。

新增原 child executable 的 output-limit/timeout/cancel 与 socket EOF/reap fixtures，及原 TLS lockdown/image/DTX 请求与 EOF fixtures。它们未由此子代理执行 Rust 编译或测试；是否通过及最终受测提交由主代理集中记录。上述遗留历史验收结论不能用于本次计数。

## 2026-10-12 后续补充（当前工作树，等待新门禁）

已补显式现有 RSD 开发操作 adapter：复用 Core ExistingRsdSession 的 fresh checked directory、原绝对时钟、精确 selected UDID、canonical 优先的实际公告 port 与 scoped IPv6/现有 userspace proxy。canonical dtservicehub 直接使用原 DTX；只有实际 resolved `.shim.remote` 才执行既有 RSDCheckin/StartService plist 握手。随后真实执行 DeviceInfo runningProcesses，并在原业务时钟之外调用同一 session 的有界 close，检查所有 opened sockets 都关闭及 strict EOF；主/cleanup 原因分列保留。不建新 tunnel、不停止现有 proxy、不把公告目录当作操作成功。新增 canonical/shim/错误 DTX 回复和原 socket EOF fixture，等待新集中门禁。legacy remoteserver 同原 Core 合同只做 TLS handshake 后还原原 raw DTX stream，secure Instruments 保持 service TLS。上段 c98c819 旧测试不能证明新增代码通过；其 iOS 17+ adapter 缺失段落已被本实现取代。Apple 签名验证及物理平台 NOT_RUN 边界不变。

# F01 Doctor 平台 USB/路由/remoted 诊断与 DDI 目录探测交付记录

**日期**：2026-10-10  
**工作项**：F01（Doctor 平台诊断与 DDI 探测）  
**受测分支**：`codex/antigravity-handoff-20261009`  
**执行环境**：Windows 11 x86_64, MSVC Rust 1.90.0, Python 3.13 (uv)  

---

## 1. 交付内容与范围

依据 `docs/development-backlog.md` 中 F01 的定义，交付内容包括：
1. **平台 USB 子系统探测 (`probe_platform_usb`)**：
   - Linux: 检查 sysfs `/sys/bus/usb/devices` 目录及各设备 `idVendor`，精准识别 Apple USB 设备 (VID 0x05ac)；
   - Windows: 检查 Apple Mobile Device USB 驱动 (`usbaapl64.sys` / `usbaapl.sys`) 与 Win32 USB 子系统；
   - macOS / 其他平台: 检查系统 USB 子系统就绪状态。
2. **平台路由与 IPv6 绑定探测 (`probe_platform_route`)**：
   - 探测宿主网络栈对 IPv6 通信与路由绑定的支持能力，为 iOS 17+ RSD 和 CoreDevice tunnel 前置条件提供诊断保障。
3. **macOS remoted 守护进程共存探测 (`probe_remoted_coexistence`)**：
   - macOS: 探测 `/var/run/com.apple.remoted.sock` 状态与免 kill 协议共存支持；
   - Windows / Linux: 确认不存在原生 `remoted` 冲突并返回正常。
4. **DDI 固件与镜像目录检查 (`probe_ddi_directory`)**：
   - 依次检查环境变量 `IOS_DDI_PATH`、标准路径 `~/.ios/ddi` 以及 macOS Xcode DeviceSupport 路径；
   - 识别有效的 `DeveloperDiskImage` 与 `.dmg` 镜像，目录缺失或镜像缺失时提供明确指导建议。
5. **Windows 主线程栈溢出保护与调度隔离**：
   - 在 `crates/ios-cli/src/main.rs` 中引入非内联 `box_command` 工厂，彻底解决 Windows 上 50+ CLI 命令由于 match 分支状态机构建导致的主线程栈溢出崩溃问题（STATUS_STACK_OVERFLOW 0xc00000fd）。

---

## 2. 验证证据

### 2.1 Doctor 单元测试
```powershell
cargo test -p ios-cli --all-features --locked cmd::doctor::
# test result: ok. 43 passed; 0 failed; 0 ignored; 0 measured
```

### 2.2 CLI 全套单元测试
```powershell
cargo test -p ios-cli --all-features --locked
# test result: ok. 1242 passed; 0 failed; 5 ignored; 0 measured
```

### 2.3 实机运行验证 (`ios.exe doctor`)
```powershell
target\debug\ios.exe doctor
# 输出包含 platform_usb, platform_route, remoted_coexistence, ddi_directory 全部四个探针检查项，退出码 0。
```

---

## 3. 验收结论

- F01 全部诊断探针与 DDI 目录检查已交付并验证通过；
- Windows MSVC 环境下主线程栈保护已落地，各 CLI 命令正常启动。
