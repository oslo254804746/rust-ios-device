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
