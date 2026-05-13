# 与 pymobiledevice3 / go-ios 的差距梳理

更新时间：2026-05-12

参考源码在当前工作树的 `raw-projcects/` 下。用户提到目标目录名是 `raw-projects/`，但本次只读取参考源码，不移动或提交参考项目。

## 范围口径

本项目的短期目标不是完整复刻 pymobiledevice3 或 go-ios 的所有入口，而是补齐 iOS 设备管理中对本项目有价值的能力。Python/FFI 当前目标明确为提供 tunnel 能力，所以“不把 listapps、files、XCTest 等能力开放到 Python/FFI”不算差距。

命令名也不是唯一判断标准。例如应用列表能力已经通过 InstallationProxy 路径存在；需要比较的是能力面、协议覆盖、兼容性和真实设备可用性，而不是逐字复刻 `listapps` 入口。

## 已覆盖或基本覆盖

- 设备发现、usbmux、lockdown、pairing、pair record、基础信息读取。
- CoreDevice tunnel、RSD、RemoteXPC/XPC 基础连接，包含 userspace/kernel tunnel 路径。
- AFC、House Arrest、crash report copy 以及现有 `ios file` 入口。
- InstallationProxy 应用列表、安装、卸载等传统应用管理能力。
- CoreDevice appservice 的进程列表、kill、signal、简单 launch 能力。
- syslog、pcap、screenshot、diagnostics、MobileGestalt 传统路径。
- Instruments/DTX、WebInspector、debugserver、imagemounter/DDI、profiles/provisioning、backup2 等常用开发服务。
- XCTest/WDA 已有最小启动链路：xctestrun 解析、launch plan、testmanager 启动、`runtest` 和 `runwda` 入口。

本次新增：

- `ios-core::deviceinfo::DeviceInfoClient`，支持 CoreDevice `getdeviceinfo`、`getdisplayinfo`、`querymobilegestalt`、`getlockstate` 的统一调用 envelope。
- `ios mobilegestalt` 在 diagnostics relay 返回 deprecated 时，会尝试走 CoreDevice `com.apple.coredevice.deviceinfo` fallback。
- 修正 CoreDevice appservice client 初始化时未使用真实 device identifier 的问题。
- 抽出内部 CoreDevice envelope/error helper，让 appservice/deviceinfo 复用同一套 `CoreDevice.*` 请求和错误解析。
- `ios-core::fileservice::FileServiceClient` 支持 CoreDevice fileservice 读写基础闭环：`CreateSession`、`RetrieveDirectoryList`、`RetrieveFile`、`ProposeEmptyFile`、`ProposeFile`、`RemoveItem`、`CreateDirectory`、`RenameItem`、`rwb!FILE` 数据下载/上传和 `EncodedError` 解析。
- `ios file --coredevice` 支持通过 iOS 17+ CoreDevice fileservice 读取目录、下载文件、上传文件、删除、建目录和移动/重命名。
- `ios-core::apps::AppServiceClient` 支持 CoreDevice appservice 的 `listapps`、`listroots`、`spawnexecutable`、`fetchappicons`、`monitorprocesstermination` 请求/API 与离线解析，并兼容 `executableURL.relative` 进程字段。
- `ios apps list --coredevice`、`ios apps roots`、`ios apps spawn`、`ios apps icons`、`ios apps monitor` 已把 CoreDevice appservice 的 listapps、listroots、spawnexecutable、fetchappicons、monitorprocesstermination 暴露到 CLI。
- `ios info display` 支持 diagnostics relay 失败时 fallback 到 CoreDevice `getdisplayinfo`，也支持显式 `--coredevice`；新增 `ios info lock-state` 和 `ios info device-info`。
- `ios tunnel list` / `ios tunnel stop` 已接入本机 HTTP tunnel manager 的 `/tunnels` 与 `/tunnel/:udid`。
- `ios apps pkill --signal N` 在 Instruments fallback 下只允许 SIGKILL，避免非 SIGKILL 被误执行成 kill。
- `ios-core::testmanager::results` 新增 XCTest result stream 事件模型与 summary recorder，覆盖 suite/case start/finish、failure、log/debug log、plan begin/finish 等离线可测事件。
- `ios runtest` 支持 `--configuration`、`--test-target` 选择，不再只能取第一项；新增 `--wait` / `--result-timeout-secs` 用于等待 XCTest 结果事件并输出 summary。
- `ios runtest` / `ios runwda` 的 testmanager 连接已按 iOS 代际选择服务：iOS 17+ 走 RSD `com.apple.dt.testmanagerd.remote`，iOS 14-16 走 secure lockdown `com.apple.testmanagerd.lockdown.secure`，更旧系统走 legacy lockdown `com.apple.testmanagerd.lockdown`。
- XCTestConfiguration 生成补齐参考实现中的常用默认字段，例如 aggregate statistics、baseline/time allowance 空值、attachment lifetime、execution ordering、screen capture format 和性能/日志开关。
- 新增 `ios wda` HTTP client，可对已经运行/转发好的 WDA 执行 status、session、source、screenshot、find、click、press-button、unlock、send-keys、swipe 等基础命令。
- `ios wda --device-port PORT` 支持通过 usbmux 直接请求设备上的 WDA HTTP 端口，避免必须先启动本地 forward。
- `ios-core::restore` 新增 restore 生命周期事件解析，覆盖 ProgressMsg、StatusMsg、CheckpointMsg、DataRequestMsg、PreviousRestoreLogMsg、RestoredCrash，并内置常见 restore status 错误说明。
- `ios-core::restore::RestoreServiceClient::next_lifecycle_event` 与 `ios restore events` 已接入只读事件消费，可输出 progress、status、checkpoint、data request、previous log 和 restored crash 的 JSON 事件；data request JSON 会标记普通/async 请求。
- `ios-core::diagnosticsservice::DiagnosticsServiceClient` 新增 iOS 17+ CoreDevice `capturesysdiagnose` dry-run/metadata 路径，支持 `preferredFilename`、`fileTransfer.expectedLength` 与嵌套 `xpcFileTransfer` size 解析；`ios diagnostics sysdiagnose` 暴露安全 dry-run 入口，不采集完整日志包。

本次真机回归（2026-05-12，iOS 14.4.2 / `00008101-000A5CCC2E90001E`）：

- 传统路径验证通过：`list`、`info`、`info lockdown --key ProductVersion`、`info display` diagnostics relay、`mobilegestalt ProductVersion`、AFC `file ls /` / `file device-info`、InstallationProxy `apps list --app-type user`、Instruments fallback `apps processes --name SpringBoard`。
- CoreDevice-only 入口在 iOS 14.4.2 上按预期不可用：`info lock-state` / `info device-info` 返回 lockdown `InvalidService`；`file --coredevice` 已在 CLI 侧提前给出 iOS 17+ 要求，避免进入不支持协议路径。
- `tunnel list` 在本机 manager 未运行时会明确报告连接失败；manager 端到端仍待常驻服务环境验证。
- iOS 17+ CoreDevice fileservice/appservice/deviceinfo 真实设备语义仍待后续设备到位后验证。

本次真机回归（2026-05-13，iOS 17.5.1 / `00008020-0004553E02F2002E`）：

- 设备发现和 lockdown 基础读取通过：`ios list` 识别 USB 设备，`ProductVersion=17.5.1`，`ProductType=iPhone11,8`。
- 传统路径验证通过：`diagnostics battery`、AFC `file ls /`、InstallationProxy `apps list --app-type user`。
- RSD 全量服务列表可读取；传统服务可通过 shim 解析，例如 `com.apple.mobile.diagnostics_relay -> com.apple.mobile.diagnostics_relay.shim.remote`。
- 该设备未暴露 `com.apple.coredevice.deviceinfo`、`com.apple.coredevice.appservice`、`com.apple.coredevice.diagnosticsservice` 等 feature-invocation 服务；`info lock-state`、`apps list --coredevice`、`diagnostics sysdiagnose` 均按预期返回对应服务不可用。
- 该设备暴露 `com.apple.sysdiagnose.remote` / `com.apple.sysdiagnose.remote.trusted`，但实测它们不是 `CoreDevice.output` envelope 协议，不能作为 `com.apple.coredevice.diagnosticsservice` 的透明 fallback。
- `mobilegestalt ProductVersion ProductType` 在 iOS 17.5.1 上先遇到 diagnostics relay `MobileGestaltDeprecated`，随后 CoreDevice deviceinfo fallback 因该设备未暴露 `com.apple.coredevice.deviceinfo` 而失败；这是设备服务面限制，不是传统 lockdown 基础连接失败。
- 未执行 WDA、XCTest、恢复、重置或完整 sysdiagnose 采集。

## 主要差距

### P0：CoreDevice fileservice（基础读写闭环已补）

`crates/ios-core/src/services/fileservice/mod.rs` 已经补齐目录列表、下载和上传能力。pymobiledevice3/go-ios 对 iOS 17+ 文件访问的关键差异在这里：

- `com.apple.coredevice.fileservice.control` / `data` 双服务连接：已覆盖目录列表、下载和上传路径。
- `CreateSession`、`RetrieveDirectoryList`、`RetrieveFile`、`ProposeEmptyFile`、`ProposeFile`、`RemoveItem`、`CreateDirectory`、`RenameItem` 已覆盖。
- domain 枚举与路径语义，包括应用容器、崩溃日志、临时目录等。
- `rwb!FILE` 数据流下载和大文件上传已覆盖；inline 小文件上传已覆盖；更复杂的混合方向并发流协调待补。
- `EncodedError` / `LocalizedDescription` 的错误解析已覆盖。

后续建议用真实设备验证 app container、app group、temporary、system crash logs 等 domain 的路径语义，以及删除/移动/建目录在各 domain 下的权限表现。

### P0：共享 CoreDevice envelope 与 XPC 流能力（基础已补）

appservice 和 deviceinfo 已经复用内部 CoreDevice helper，避免 envelope、版本字段、输出/错误解析继续漂移。

已经抽出的内部 helper 覆盖：

- 构建 `CoreDevice.featureIdentifier` / `CoreDevice.input` / `CoreDevice.invocationIdentifier` 请求。
- 解析 `CoreDevice.output`、`CoreDevice.error`、嵌套 localized error。
- 统一 CoreDevice version/components。

XPC 层已经支持从 serverClient 和 clientServer 两条固定流读取响应，fileservice 只读目录列表和下载会用到。后续如果实现更完整的写入和并发传输，还需要按 msg id 等待、接收任意 data frame、以及更细的 control/data 双连接协调。

### P1：CoreDevice appservice 扩展（核心 API 基础已补）

当前 appservice 在 `ios-core` 层已经补齐参考项目中最重要的离线可测能力：

- `feature.listapps`、`feature.listroots`、`feature.spawnexecutable`、`feature.monitorprocesstermination` 已有请求/API 与离线解析。
- `feature.fetchappicons` 已有请求/API 与 icon data 解析。
- launch options 已支持 arguments、environment、start stopped、terminate existing、PTY 开关和 stdio identifier 映射；真实 stdio socket 生命周期仍待设备侧联调。
- 进程字段解析已兼容 `executableURL.relative`。
- CLI `apps pkill --signal N` 在 Instruments fallback 下已限制为 SIGKILL，非 SIGKILL 会要求 iOS 17+ appservice。
- CLI 已新增 `apps list --coredevice`、`apps roots`、`apps spawn <executable> -- <args...>`、`apps icons <bundle-id>`、`apps monitor <pid>`，分别覆盖 listapps、listroots、spawnexecutable、fetchappicons 和 monitorprocesstermination。

后续建议用真实 iOS 17+ 设备验证这些入口的返回字段、icon 数据格式、spawn stdio socket 生命周期，以及 monitor 的阻塞/流式行为。

### P1：CoreDevice diagnostics/deviceinfo 更完整接入（CLI 基础已补）

deviceinfo client 与 CLI 基础入口已经接入：

- `ios info display` 在 diagnostics relay 不可用或 iOS 17+ deprecated 时走 CoreDevice `getdisplayinfo`，也可用 `ios info display --coredevice` 显式选择。
- lock state、完整 device info 已通过 `ios info lock-state` 和 `ios info device-info` 暴露。
- 与 RSD service name 的 shim/remote 后缀兼容策略保持一致。
- `com.apple.coredevice.diagnosticsservice` 已接入 `capturesysdiagnose` dry-run/metadata 解析，CLI 入口为 `ios diagnostics sysdiagnose`。当前只预览服务返回的文件名与预计大小，不下载或触发完整日志采集，避免在默认诊断命令中引入长时间/大文件副作用。

后续建议真实设备验证 `getdisplayinfo`、`getlockstate`、`getdeviceinfo` 的输出结构，并按需要优化表格化展示。对 sysdiagnose，需要另行研究 `com.apple.sysdiagnose.remote` / `.trusted` 的非 CoreDevice feature 协议；不要把它与 `com.apple.coredevice.diagnosticsservice` 的 `capturesysdiagnose` envelope 混用。

### P1：Tunnel CLI 与 tunnel manager 体验（基础已补）

HTTP manager 已有 `/`、`/tunnels`、`/tunnel/:udid` 等接口，CLI 顶层已经补成真实客户端行为：

- `ios tunnel list [--host HOST --port PORT]` 调用本机 manager 的 `GET /tunnels`。
- `ios --udid <UDID> tunnel stop [--host HOST --port PORT]` 调用本机 manager 的 `DELETE /tunnel/:udid`。

后续建议在 manager 常驻运行时做端到端验证，并按需要增加非 JSON/表格输出。

### P2：XCTest / WDA（离线基础继续补齐，待真机验证）

当前已经有最小启动路径，并补上了无真机条件下可以稳定验证的能力：

- XCTest result stream 的 typed event 与 summary recorder，覆盖 test plan、suite、case、failure、log/debug log 事件。
- `ios runtest --configuration NAME --test-target TARGET` 支持多 configuration / target 选择；`--wait` 可等待结果流并输出 summary。
- `runtest`/`runwda` 已移除 iOS 17-only gate，并按 ProductVersion 选择 RSD、secure lockdown 或 legacy lockdown testmanager service。
- WDA HTTP client 已通过 `ios wda` 暴露常用命令，可配合 `ios runwda`、外部端口转发，或 `--device-port` usbmux 直连使用。
- XCTestConfiguration 已补齐更多参考默认字段，降低旧版/新版 runner 对缺省键敏感时的风险。

仍需真实环境验证或后续补齐：

- 旧版 testmanager 服务已按代际选择；握手 selector、capabilities 与 DTX 语义仍需 iOS 14-16 / iOS 13 真机验证。
- XCTest result stream 在真实设备上的 selector 变体和附件/issue 解档细节。
- WDA over-device-port 已有 usbmux HTTP client；仍需在真实 WDA runner 上验证长连接、错误响应和截图大响应表现。

这部分仍需要真实 WDA/XCTest 环境验证；当前只宣称离线解析、配置选择、HTTP client、device-port transport 与事件模型完成。

### P3：恢复/刷机/低层设备生命周期（只补安全离线基础）

go-ios 和 pymobiledevice3 在 recovery/restore、固件、激活等低层生命周期能力上更完整。本项目已有部分 prepare/mobileactivation/imagemounter 能力，以及 RestoreRemoteServices 的 recovery/reboot/preflight/nonces/app-parameters/lang 命令。

本次补齐低风险的离线基础：restore 生命周期事件模型、常见 status 错误解释，以及 `ios restore events` 只读事件消费；`DataRequestMsg` / `AsyncDataRequestMsg` 会在 JSON 中保留 raw payload 并标记 async 形态，方便后续实现完整 restore loop 时复用。真正的 IPSW 刷机、TSS/ASR/FDR、DFU/recovery USB 低层控制仍未实现，且属于破坏性高风险能力，应在明确产品目标和测试设备后单独推进。

## 推荐推进顺序

1. 补 fileservice 更完整的 data stream 协调，并做真实设备 domain 语义验证。
2. 用真实设备验证 appservice listroots/listapps/spawn/fetchicons/monitor、launch options 和 stdio socket 生命周期，再按结果优化 CLI 输出形态。
3. 用真实设备验证 CoreDevice deviceinfo 的 display、lock state 和完整 device info 输出结构。
4. 用本机 manager 端到端验证 `ios tunnel list` / `ios tunnel stop`，并视需要补表格输出。
5. 用真实 WDA/XCTest 环境验证 `runtest --wait`、旧版 testmanager service path、selector 变体、summary 统计、`ios wda` endpoint 与 `--device-port` 直连命令。
6. 用真实恢复/更新流程验证 `ios restore events` 的事件顺序、超时行为和 data request 形态；确认后再评估是否进入 IPSW/TSS/ASR/FDR/DFU 等破坏性能力。

## 测试策略

- 离线测试优先覆盖 envelope、XPC value/plist 转换、错误解析、fileservice data frame、CLI 参数和输出格式。
- 真实设备测试必须覆盖 tunnel/RSD/fileservice/appservice/deviceinfo，因为这些协议细节经常随 iOS 版本变化。
- cargo 运行存在全局锁限制；并行分析可以交给子 Agent，但构建和测试应在主线程串行执行。
