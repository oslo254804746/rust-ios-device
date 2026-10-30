# iOS 14.4.2 CLI 入口覆盖清单（补测后）

复核完整帮助树/历史清单 291 项，合并三轮 iOS 14 实际业务证据：161 项有调用记录，比首轮 133 项补充 28 项。**调用、exit 0、空结果及受控中止都不等于功能通过。** SDK 25 项直接检查另计，不混入 CLI 数量。

详细产物、失败/空结果和验证边界见 [覆盖复核报告](ios14-coverage-audit-2026-10-30.md) 与 [问题修复报告](ios14-fixes-2026-10-30.md)。调用次数截至合并快照，后续同入口复测另见报告，不作全部调用次数统计。参数分支只列本轮已核实范围，不能代表所有参数组合、并发及所有 iOS 14–16 版本。

未调用的 130 项：当前版本不适用 42 项、主机管理器 3 项、状态/数据变更 67 项、缺素材/凭据/硬件或完整页面 17 项、未实现 1 项。没有将未调用入口计为通过。

| 操作入口 | 调用/分类 | 实际结论或未执行原因 |
| --- | --- | --- |
| `ios accessibility-audit audit-types` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios accessibility-audit capabilities` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios accessibility-audit describe` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios accessibility-audit list-items` | 状态/数据变更，本轮未执行 | This handler moves UI focus and changes accessibility visuals while listing items. |
| `ios accessibility-audit navigate` | 状态/数据变更，本轮未执行 | This handler navigates accessibility focus/content in the active app. |
| `ios accessibility-audit run-audit` | 调用 1 次；exit 0 × 1 | CLI exit 0; audit result was complete with zero issues (empty result is the observed result). |
| `ios accessibility-audit settings` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios accessibility-audit tap` | 状态/数据变更，本轮未执行 | This handler injects a tap into the active app. |
| `ios activation activate` | 状态/数据变更，本轮未执行 | This handler applies an activation record or contacts the activation service and changes device activation state. |
| `ios activation deactivate` | 状态/数据变更，本轮未执行 | This handler deactivates the device. |
| `ios activation info` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios activation itunes-activate` | 状态/数据变更，本轮未执行 | This legacy activation helper changes the device activation state. |
| `ios activation session-info` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios activation state` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios amfi enable-developer-mode` | 当前14.4.2版本不适用 | Requires iOS 16 or later; iOS 14.4.2 is not applicable. |
| `ios amfi reveal-developer-mode` | 当前14.4.2版本不适用 | Requires iOS 16 or later; iOS 14.4.2 is not applicable. |
| `ios apps archive` | 状态/数据变更，本轮未执行 | InstallationProxy archive changes installed-app state on device. |
| `ios apps icons` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios apps install` | 状态/数据变更，本轮未执行 | InstallationProxy installs an app on device. |
| `ios apps install-record` | 当前14.4.2版本不适用 | handler走iOS 17+ CoreDevice/RSD路径，iOS 14.4.2不适用 |
| `ios apps kill` | 状态/数据变更，本轮未执行 | AppService terminates a device process. |
| `ios apps launch` | 状态/数据变更，本轮未执行 | AppService starts an app on device. |
| `ios apps list` | 调用 5 次；exit 0 × 5 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios apps monitor` | 当前14.4.2版本不适用 | handler走iOS 17+ CoreDevice/RSD路径，iOS 14.4.2不适用 |
| `ios apps pkill` | 状态/数据变更，本轮未执行 | AppService terminates matching device processes. |
| `ios apps processes` | 当前14.4.2版本不适用 | handler走iOS 17+ CoreDevice/RSD路径，iOS 14.4.2不适用 |
| `ios apps restore` | 状态/数据变更，本轮未执行 | InstallationProxy restores an archived app on device. |
| `ios apps roots` | 当前14.4.2版本不适用 | handler走iOS 17+ CoreDevice/RSD路径，iOS 14.4.2不适用 |
| `ios apps show` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios apps signal` | 状态/数据变更，本轮未执行 | AppService sends a process signal on device. |
| `ios apps spawn` | 状态/数据变更，本轮未执行 | AppService starts a device process. |
| `ios apps uninstall` | 状态/数据变更，本轮未执行 | InstallationProxy removes an installed app from device. |
| `ios apps upgrade` | 状态/数据变更，本轮未执行 | InstallationProxy upgrades an installed app on device. |
| `ios arbitration check-in` | 调用 1 次；exit 0 × 1 | Own unique hostname, no force flag; check-in exited 0. |
| `ios arbitration check-out` | 调用 1 次；exit 0 × 1 | Matching own check-out exited 0. |
| `ios arbitration version` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios backup change-password` | 状态/数据变更，本轮未执行 | MobileBackup2 changes device backup encryption credentials/settings. |
| `ios backup create` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios backup encryption` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios backup encryption-status` | 调用 7 次；exit 0 × 7 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios backup extract` | 调用 2 次；exit 0 × 1、exit 1 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios backup extract-device` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios backup info` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios backup list` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios backup list-local` | 调用 3 次；exit 0 × 2、exit 2 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios backup restore` | 状态/数据变更，本轮未执行 | MobileBackup2 restores backup data onto device and may reboot it. |
| `ios backup unback` | 调用 1 次；exit 1 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios backup unback-device` | 调用 1 次；exit 1 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios backup version` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios batterycheck` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios batteryregistry` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios btlogger capture` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios companion forward` | 缺凭据/硬件/受控目标或素材 | Requires a configured companion/relay endpoint and an available paired companion service. |
| `ios companion get` | 缺凭据/硬件/受控目标或素材 | Requires a configured companion/relay endpoint and an available paired companion service. |
| `ios companion list` | 调用 2 次；exit 0 × 2 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios companion listen` | 调用 1 次；exit 1 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios companion stop` | 缺凭据/硬件/受控目标或素材 | Requires an active companion operation to stop. |
| `ios crash clear` | 状态/数据变更，本轮未执行 | The handler removes crash reports below a device path. |
| `ios crash flush` | 调用 1 次；exit 0 × 1 | CLI exit 0. Handler flushes pending products; this is not a clear operation. |
| `ios crash ls` | 调用 3 次；exit 0 × 3 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios crash parse` | 调用 285 次；exit 0 × 284、exit 1 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios crash parse-latest` | 调用 3 次；exit 0 × 2、exit 1 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios crash pull` | 调用 4 次；exit 0 × 4 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios crash pull-all` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios crash rm` | 状态/数据变更，本轮未执行 | The handler removes matching crash reports from device storage. |
| `ios crash show` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios crash sysdiagnose` | 当前入口未实现 | CLI source explicitly says crash-report sysdiagnose collection is not implemented. |
| `ios crash watch` | 调用 1 次；exit 1 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios ddi auto` | 调用 3 次；exit 0 × 3 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios ddi cryptex auto-install` | 当前14.4.2版本不适用 | handler连接RSD并调用Cryptex自动个性化安装；只适用于Cryptex现代系统，不属于iOS 14.4.2路径 |
| `ios ddi cryptex list` | 当前14.4.2版本不适用 | handler走iOS 17+ CoreDevice/RSD路径，iOS 14.4.2不适用 |
| `ios ddi cryptex nonce` | 当前14.4.2版本不适用 | handler走iOS 17+ CoreDevice/RSD路径，iOS 14.4.2不适用 |
| `ios ddi cryptex personalization-identifiers` | 当前14.4.2版本不适用 | handler走iOS 17+ CoreDevice/RSD路径，iOS 14.4.2不适用 |
| `ios ddi cryptex uninstall` | 当前14.4.2版本不适用 | handler通过Cryptex RSD删除指定已安装镜像；只适用于Cryptex现代系统，不属于iOS 14.4.2路径 |
| `ios ddi devices` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios ddi devmode-status` | 当前14.4.2版本不适用 | Requires iOS 16 or later; iOS 14.4.2 is not applicable. |
| `ios ddi list` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios ddi lookup` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios ddi mount` | 调用 3 次；exit 0 × 3 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios ddi nonce` | 当前14.4.2版本不适用 | handler走iOS 17+ CoreDevice/RSD路径，iOS 14.4.2不适用 |
| `ios ddi personalization-identifiers` | 当前14.4.2版本不适用 | handler走iOS 17+ CoreDevice/RSD路径，iOS 14.4.2不适用 |
| `ios ddi personalization-manifests` | 当前14.4.2版本不适用 | handler走iOS 17+ CoreDevice/RSD路径，iOS 14.4.2不适用 |
| `ios ddi status` | 调用 17 次；exit 0 × 17 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios ddi unmount` | 调用 6 次；exit 0 × 6 | Final unmount exited 0; subsequent status reported mounted=false. |
| `ios debug` | 当前14.4.2版本不适用 | 该调试入口面向iOS 17+现代服务，目标iOS 14.4.2不适用 |
| `ios debugserver send` | 调用 5 次；exit 0 × 3、exit 1 × 2 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios device-control configuration get` | 当前14.4.2版本不适用 | handler走iOS 17+ CoreDevice/RSD路径，iOS 14.4.2不适用 |
| `ios device-control configuration set` | 当前14.4.2版本不适用 | handler走iOS 17+ CoreDevice/RSD路径，iOS 14.4.2不适用 |
| `ios device-control display audio` | 当前14.4.2版本不适用 | handler走iOS 17+ CoreDevice/RSD路径，iOS 14.4.2不适用 |
| `ios device-control display status` | 当前14.4.2版本不适用 | handler走iOS 17+ CoreDevice/RSD路径，iOS 14.4.2不适用 |
| `ios device-control display video` | 当前14.4.2版本不适用 | handler走iOS 17+ CoreDevice/RSD路径，iOS 14.4.2不适用 |
| `ios device-control orientation` | 当前14.4.2版本不适用 | handler走iOS 17+ CoreDevice/RSD路径，iOS 14.4.2不适用 |
| `ios devicestate disable` | 状态/数据变更，本轮未执行 | The handler disables the active device condition profile. |
| `ios devicestate enable` | 状态/数据变更，本轮未执行 | The handler enables a device condition profile. |
| `ios devicestate list` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios diagnostics battery` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios diagnostics battery-monitor` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios diagnostics gasgauge` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios diagnostics hdmi` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios diagnostics ioregistry` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios diagnostics list` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios diagnostics nand` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios diagnostics reboot` | 调用 5 次；exit 0 × 4、exit 1 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios diagnostics show` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios diagnostics shutdown` | 状态/数据变更，本轮未执行 | The handler requests device shutdown. |
| `ios diagnostics sysdiagnose` | 当前14.4.2版本不适用 | handler走iOS 17+ CoreDevice/RSD路径，iOS 14.4.2不适用 |
| `ios diagnostics wifi` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios discover mdns` | 当前14.4.2版本不适用 | handler走iOS 17+ CoreDevice/RSD路径，iOS 14.4.2不适用 |
| `ios discover mobdev2` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios discover paired-identity` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios discover remotepairing` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios diskspace` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios doctor` | 调用 2 次；exit 1 × 2 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios dproxy service` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios erase` | 状态/数据变更，本轮未执行 | The handler requests device erase and reboot. |
| `ios file device-info` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios file ls` | 调用 16 次；exit 0 × 14、exit 1 × 2 | Separate branches: House Arrest VendDocuments `/Documents` returned 4 entries; VendContainer returned InstallationLookupFailed; VendDocuments root `/` was denied. AFC root was listed, and final cleanup confirmed the sentinel absent. |
| `ios file mkdir` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios file mv` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios file pull` | 调用 2 次；exit 0 × 2 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios file push` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios file rm` | 调用 2 次；exit 0 × 2 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios file stat` | 调用 2 次；exit 0 × 2 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios file tree` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios file-relay` | 调用 1 次；exit 1 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios forward` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios heartbeat` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios hid button` | 状态/数据变更，本轮未执行 | The handler injects a hardware-button event. |
| `ios hid key` | 状态/数据变更，本轮未执行 | The handler injects a keyboard key press/release. |
| `ios hid observe-stop` | 状态/数据变更，本轮未执行 | The observer performs the protocol stopAll media operation. |
| `ios hid swipe` | 状态/数据变更，本轮未执行 | The handler injects touch/swipe reports. |
| `ios hid tap` | 状态/数据变更，本轮未执行 | The handler injects touchscreen reports. |
| `ios hid text` | 状态/数据变更，本轮未执行 | The handler types text through a virtual keyboard. |
| `ios hid touch` | 状态/数据变更，本轮未执行 | The handler injects a touch state transition. |
| `ios httpproxy remove` | 状态/数据变更，本轮未执行 | The handler removes the tool's device-wide HTTP proxy profile. |
| `ios httpproxy set` | 状态/数据变更，本轮未执行 | The handler installs a device-wide proxy profile that redirects app traffic. |
| `ios idam get` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios idam set` | 状态/数据变更，本轮未执行 | The handler changes the device IDAM value. |
| `ios info` | 调用 30 次；exit 0 × 13、exit 1 × 17 | Final ProductVersion query exited 0; IsSupervised query was attempted but exited 1, so that value remains unknown. |
| `ios info device-info` | 当前14.4.2版本不适用 | handler走iOS 17+ CoreDevice/RSD路径，iOS 14.4.2不适用 |
| `ios info display` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios info lock-state` | 当前14.4.2版本不适用 | handler走iOS 17+ CoreDevice/RSD路径，iOS 14.4.2不适用 |
| `ios info lockdown` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios instruments apps` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios instruments cpu` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios instruments energy` | 调用 1 次；exit 0 × 1 | CLI exit 0; energy payload was empty, so no nonempty energy sample was validated. |
| `ios instruments fps` | 调用 15 次；exit 0 × 10、exit 1 × 5 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios instruments gpu` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios instruments har` | 调用 1 次；exit 0 × 1 | CLI exit 0; an ordinary CoreAnimation event appeared but no HTTP material was available. |
| `ios instruments kdebug` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios instruments kill` | 状态/数据变更，本轮未执行 | The handler terminates a device app/process. |
| `ios instruments launch` | 状态/数据变更，本轮未执行 | The handler launches an app on device. |
| `ios instruments network` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios instruments notifications` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios instruments procattrs` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios instruments ps` | 调用 2 次；exit 0 × 2 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios instruments sysattrs` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios instruments sysmon-process` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios instruments sysmon-threshold` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios instruments trace` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios ip` | 调用 1 次；exit 1 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios list` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios listen` | 调用 1 次；exit 1 × 1 | Attached-device event observed; listener was host-stopped, so natural detach completion was not observed. |
| `ios location gpx` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios location reset` | 调用 8 次；exit 0 × 8 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios location set` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios lockdown assistive-touch` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios lockdown date` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios lockdown developer-mode` | 当前14.4.2版本不适用 | Requires iOS 16 or later; iOS 14.4.2 is not applicable. |
| `ios lockdown device-name` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios lockdown font-size` | 调用 1 次；exit 1 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios lockdown get` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios lockdown heartbeat` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios lockdown info` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios lockdown invert-display` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios lockdown language` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios lockdown locale` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios lockdown remove` | 状态/数据变更，本轮未执行 | The handler removes a lockdown key from device settings. |
| `ios lockdown reset-accessibility` | 状态/数据变更，本轮未执行 | The handler resets accessibility lockdown toggles on device. |
| `ios lockdown save-pair-record` | 调用 1 次；exit 0 × 1 | CLI exit 0; local export matched the existing default pair record. No pairing/trust change. |
| `ios lockdown set` | 状态/数据变更，本轮未执行 | The handler writes a lockdown value to device settings. |
| `ios lockdown start-tunnel` | 当前14.4.2版本不适用 | handler走iOS 17+ CoreDevice/RSD路径，iOS 14.4.2不适用 |
| `ios lockdown time-format` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios lockdown time-zone` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios lockdown voice-over` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios lockdown wifi-connections` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios lockdown zoom-touch` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios mdm clear-passcode` | 状态/数据变更，本轮未执行 | The handler removes the device lock passcode using an unlock token. |
| `ios mdm clear-screen-time-password` | 状态/数据变更，本轮未执行 | The handler removes the Screen Time passcode. |
| `ios mdm fetch-unlock-token` | 缺凭据/硬件/受控目标或素材 | Requires an already supervised device and supervisor P12; do not create supervision credentials for this check. |
| `ios mdm passcode-present` | 缺凭据/硬件/受控目标或素材 | Read-only request requires an already supervised device and supervisor P12. |
| `ios mdm security-info` | 缺凭据/硬件/受控目标或素材 | Read-only request requires an already supervised device and supervisor P12. |
| `ios memlimitoff` | 状态/数据变更，本轮未执行 | The handler disables the memory limit for a device process. |
| `ios mobilegestalt` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios notify observe` | 调用 2 次；exit 0 × 1、exit 1 × 1 | Independent controlled validation matched 2 of 2 requested events. |
| `ios notify post` | 调用 1 次；exit 0 × 1 | Controlled notification post; CLI exit 0. |
| `ios notify roundtrip` | 调用 1 次；exit 0 × 1 | Controlled self-posted event returned; CLI exit 0. |
| `ios notify springboard` | 调用 1 次；exit 1 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios notify stream` | 调用 2 次；exit 0 × 2 | Independent controlled validation matched 2 of 2 requested events. |
| `ios notify wait` | 调用 2 次；exit 0 × 1、exit 1 × 1 | Independent controlled validation matched 1 of 1 requested events. |
| `ios os-trace archive` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios os-trace collect` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios os-trace ps` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios os-trace stream` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios pair` | 状态/数据变更，本轮未执行 | The command establishes pairing trust and saves new device credentials. |
| `ios pair show-record` | 调用 4 次；exit 0 × 4 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios pair supervised` | 状态/数据变更，本轮未执行 | The command performs supervised pairing and writes pair/escrow credentials. |
| `ios pair wifi` | 状态/数据变更，本轮未执行 | The command establishes remote Wi-Fi pairing and saves remote credentials. |
| `ios pasteboard export` | 当前14.4.2版本不适用 | handler走iOS 17+ CoreDevice/RSD路径，iOS 14.4.2不适用 |
| `ios pasteboard get` | 当前14.4.2版本不适用 | handler走iOS 17+ CoreDevice/RSD路径，iOS 14.4.2不适用 |
| `ios pasteboard resolve` | 当前14.4.2版本不适用 | handler走iOS 17+ CoreDevice/RSD路径，iOS 14.4.2不适用 |
| `ios pasteboard set` | 状态/数据变更，本轮未执行 | The handler replaces device pasteboard contents. |
| `ios pasteboard watch` | 当前14.4.2版本不适用 | handler走iOS 17+ CoreDevice/RSD路径，iOS 14.4.2不适用 |
| `ios pcap` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios power-assert` | 调用 1 次；exit 0 × 1 | Two-second assertion request exited 0 and returned {}; this does not prove the OS honored the sleep-prevention effect. |
| `ios preboard commit` | 状态/数据变更，本轮未执行 | The handler commits a device stashbag manifest. |
| `ios preboard create` | 状态/数据变更，本轮未执行 | The handler creates a device stashbag manifest. |
| `ios prepare` | 状态/数据变更，本轮未执行 | The handler changes setup, supervision, language/locale/time and device marker state. |
| `ios prepare create-cert` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios profiles add` | 状态/数据变更，本轮未执行 | The handler installs a configuration profile on device. |
| `ios profiles cloud-config` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios profiles list` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios profiles remove` | 状态/数据变更，本轮未执行 | The handler removes a configuration profile from device. |
| `ios profiles show` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios profiles stored` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios provisioning dump` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios provisioning export` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios provisioning install` | 状态/数据变更，本轮未执行 | The handler installs a provisioning profile on device. |
| `ios provisioning list` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios provisioning remove` | 状态/数据变更，本轮未执行 | The handler removes a provisioning profile from device. |
| `ios provisioning show` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios restore app-parameters` | 当前14.4.2版本不适用 | 该查询经RestoreService/RSD现代服务访问；不属于iOS 14.4.2可测路径 |
| `ios restore delay-recovery-image` | 状态/数据变更，本轮未执行 | The handler changes restore/recovery boot behavior. |
| `ios restore enter-recovery` | 状态/数据变更，本轮未执行 | The handler reboots the device into recovery mode. |
| `ios restore events` | 当前14.4.2版本不适用 | 该查询经RestoreService/RSD现代服务访问；不属于iOS 14.4.2可测路径 |
| `ios restore nonces` | 当前14.4.2版本不适用 | 该查询经RestoreService/RSD现代服务访问；不属于iOS 14.4.2可测路径 |
| `ios restore preflight-info` | 当前14.4.2版本不适用 | 该查询经RestoreService/RSD现代服务访问；不属于iOS 14.4.2可测路径 |
| `ios restore preflight-requests` | 当前14.4.2版本不适用 | 该查询经RestoreService/RSD现代服务访问；不属于iOS 14.4.2可测路径 |
| `ios restore reboot` | 状态/数据变更，本轮未执行 | The handler reboots the device via RestoreService. |
| `ios restore restore-lang` | 状态/数据变更，本轮未执行 | The handler writes the restore language setting. |
| `ios restore update` | 当前14.4.2版本不适用 | Uses RestoreService over RSD; the supported `--dry-run` path also requires a local IPSW and optional TSS. |
| `ios rsd check` | 当前14.4.2版本不适用 | handler走iOS 17+ CoreDevice/RSD路径，iOS 14.4.2不适用 |
| `ios rsd observe` | 当前14.4.2版本不适用 | handler走iOS 17+ CoreDevice/RSD路径，iOS 14.4.2不适用 |
| `ios rsd services` | 当前14.4.2版本不适用 | handler走iOS 17+ CoreDevice/RSD路径，iOS 14.4.2不适用 |
| `ios runtest` | 缺凭据/硬件/受控目标或素材 | No approved test bundle/plan was present in the current fixture inventory. |
| `ios runwda` | 缺凭据/硬件/受控目标或素材 | No approved WDA runner bundle was present in the current fixture inventory. |
| `ios runxctest` | 缺凭据/硬件/受控目标或素材 | No approved XCTest runner bundle was present in the current fixture inventory. |
| `ios screenshot` | 调用 2 次；exit 0 × 1、exit 124 × 1 | `--stream` branch validated an HTTP 200 multipart PNG (6,383,003 bytes); host forced stop (exit 124), so no natural server exit. |
| `ios springboard find` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios springboard icon` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios springboard icon-all` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios springboard icon-metrics` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios springboard icons` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios springboard orientation` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios springboard state` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios springboard state-set` | 状态/数据变更，本轮未执行 | The handler replaces SpringBoard icon layout state. |
| `ios springboard wallpaper-home-screen` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios springboard wallpaper-info` | 调用 2 次；exit 0 × 1、exit 2 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios springboard wallpaper-preview-image` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios symbols download` | 调用 2 次；exit 0 × 1、exit 1 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios symbols list` | 调用 2 次；exit 0 × 1、exit 1 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios symbols pull` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios syslog` | 调用 1 次；exit 0 × 1 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios tunnel auth check` | 调用 1 次；exit 0 × 1 | Host-only policy/token check succeeded; source does not connect to a device and this is not an iOS 17 feature. |
| `ios tunnel auth create` | 调用 1 次；exit 0 × 1 | Host-only policy/token file creation succeeded; source does not connect to a device and this is not an iOS 17 feature. |
| `ios tunnel list` | 主机管理器，非本轮旧系统设备服务 | 主机隧道管理器入口，不是旧系统设备服务；list/stop需受控管理器与隧道对象，serve会自动扫描设备。本次限定指定iOS14手机，未启动全设备扫描或创建17+隧道对象。 |
| `ios tunnel serve` | 主机管理器，非本轮旧系统设备服务 | 主机隧道管理器入口，不是旧系统设备服务；list/stop需受控管理器与隧道对象，serve会自动扫描设备。本次限定指定iOS14手机，未启动全设备扫描或创建17+隧道对象。 |
| `ios tunnel start` | 当前14.4.2版本不适用 | handler走iOS 17+ CoreDevice/RSD路径，iOS 14.4.2不适用 |
| `ios tunnel stop` | 主机管理器，非本轮旧系统设备服务 | 主机隧道管理器入口，不是旧系统设备服务；list/stop需受控管理器与隧道对象，serve会自动扫描设备。本次限定指定iOS14手机，未启动全设备扫描或创建17+隧道对象。 |
| `ios viewer` | 调用 1 次；exit 0 × 1 | No-subcommand default web branch returned valid frame/stream PNGs, acknowledged POST /stop, and exited naturally (0). |
| `ios viewer clipboard` | 状态/数据变更，本轮未执行 | The handler synchronizes clipboard contents between host and device. |
| `ios viewer vnc` | 调用 4 次；exit 1 × 2、exit 124 × 1、exit 3221225725 × 1 | A direct RFB probe received a valid 1125x2436 RAW framebuffer (10,962,000 bytes), then host-forced stop (124); natural cleanup was not observed. Other VNC probes failed. |
| `ios viewer web` | 调用 3 次；exit 0 × 1、exit 1 × 1、exit 3221225725 × 1 | Final explicit web branch returned valid frame/stream PNGs, acknowledged POST /stop, and exited naturally (0); earlier attempts failed. |
| `ios wda click` | 状态/数据变更，本轮未执行 | WebDriver click changes the foreground app UI. |
| `ios wda find` | 缺凭据/硬件/受控目标或素材 | Requires a live WDA service and a controlled foreground app; no WDA bundle was present. |
| `ios wda press-button` | 状态/数据变更，本轮未执行 | WebDriver press-button changes device/UI state. |
| `ios wda screenshot` | 缺凭据/硬件/受控目标或素材 | Requires a live WDA service/session; no WDA bundle was present. |
| `ios wda send-keys` | 状态/数据变更，本轮未执行 | WebDriver injects keyboard input into the foreground app. |
| `ios wda session` | 缺凭据/硬件/受控目标或素材 | Requires a live WDA service/session; no WDA bundle was present. |
| `ios wda source` | 缺凭据/硬件/受控目标或素材 | Requires a live WDA service/session and app page; no WDA bundle was present. |
| `ios wda status` | 缺凭据/硬件/受控目标或素材 | Requires a live WDA service; no WDA bundle was present. |
| `ios wda swipe` | 状态/数据变更，本轮未执行 | WebDriver injects a swipe into the foreground app. |
| `ios wda unlock` | 状态/数据变更，本轮未执行 | WebDriver attempts to unlock the device. |
| `ios webinspector cdp` | 调用 2 次；exit 124 × 2 | `/json/version` returned 200; `/json/list` returned 500. PyMD and Rust both saw only lockdownd connected and no Listing response; this is incomplete discovery, not empty-target success. |
| `ios webinspector eval` | 缺凭据/硬件/受控目标或素材 | 可选择只读表达式，但当前Rust和参考实现均缺完整Listing，不能选择完整的设备页面目标；待可检查页面素材。 |
| `ios webinspector js-shell` | 缺凭据/硬件/受控目标或素材 | 可选择只读表达式，但当前Rust和参考实现均缺完整Listing，不能选择完整的设备页面目标；待可检查页面素材。 |
| `ios webinspector launch` | 状态/数据变更，本轮未执行 | WebInspector launches an app and can navigate to a URL. |
| `ios webinspector opened-tabs` | 调用 2 次；exit 0 × 2 | At least one direct ios*.exe invocation is evidenced; exit status is recorded separately and is not a functional pass. |
| `ios webinspector selenium` | 缺凭据/硬件/受控目标或素材 | Requires a live inspectable Safari/WebView target; the device probe observed lockdownd only and no listing response. |
| `ios wifi install` | 状态/数据变更，本轮未执行 | The handler installs a Wi-Fi configuration on device. |
| `ios wifi remove` | 状态/数据变更，本轮未执行 | The handler removes a Wi-Fi configuration from device. |
