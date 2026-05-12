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
- `ios-core::fileservice::FileServiceClient` 支持 CoreDevice fileservice 读写基础闭环：`CreateSession`、`RetrieveDirectoryList`、`RetrieveFile`、`ProposeEmptyFile`、`ProposeFile`、`rwb!FILE` 数据下载/上传和 `EncodedError` 解析。
- `ios file --coredevice` 支持通过 iOS 17+ CoreDevice fileservice 读取目录、下载文件和上传文件。

## 主要差距

### P0：CoreDevice fileservice（读与上传已补，删除/移动待补）

`crates/ios-core/src/services/fileservice/mod.rs` 已经补齐目录列表、下载和上传能力。pymobiledevice3/go-ios 对 iOS 17+ 文件访问的关键差异在这里：

- `com.apple.coredevice.fileservice.control` / `data` 双服务连接：已覆盖只读路径。
- `CreateSession`、`RetrieveDirectoryList`、`RetrieveFile`、`ProposeEmptyFile`、`ProposeFile` 已覆盖；删除/移动等写操作待补。
- domain 枚举与路径语义，包括应用容器、崩溃日志、临时目录等。
- `rwb!FILE` 数据流下载和大文件上传已覆盖；inline 小文件上传已覆盖；更复杂的混合方向并发流协调待补。
- `EncodedError` / `LocalizedDescription` 的错误解析已覆盖。

后续建议补删除/移动等写操作，并用真实设备验证 app container、app group、temporary、system crash logs 等 domain 的路径语义。

### P0：共享 CoreDevice envelope 与 XPC 流能力（基础已补）

appservice 和 deviceinfo 已经复用内部 CoreDevice helper，避免 envelope、版本字段、输出/错误解析继续漂移。

已经抽出的内部 helper 覆盖：

- 构建 `CoreDevice.featureIdentifier` / `CoreDevice.input` / `CoreDevice.invocationIdentifier` 请求。
- 解析 `CoreDevice.output`、`CoreDevice.error`、嵌套 localized error。
- 统一 CoreDevice version/components。

XPC 层已经支持从 serverClient 和 clientServer 两条固定流读取响应，fileservice 只读目录列表和下载会用到。后续如果实现更完整的写入和并发传输，还需要按 msg id 等待、接收任意 data frame、以及更细的 control/data 双连接协调。

### P1：CoreDevice appservice 扩展

当前 appservice 覆盖了最核心的 process/kill/signal/launch，但比参考项目还少：

- `feature.listapps`、`feature.listroots`、`feature.spawnexecutable`、`feature.monitorprocesstermination`。
- `feature.fetchappicons`；项目里已有 SpringBoard 图标路径，但没有 CoreDevice appservice 图标路径。
- launch 参数较窄，缺少 arguments、environment、start stopped、terminate existing、stdio socket/pty 等完整选项。
- 进程字段解析还应兼容 `executableURL.relative` 等形态。
- CLI `apps pkill --signal N` 在 Instruments fallback 下仍按 kill 语义处理，非 SIGKILL 时应避免误报或改用支持 signal 的路径。

### P1：CoreDevice diagnostics/deviceinfo 更完整接入

deviceinfo client 已经有最小实现，但 CLI 只接入了 MobileGestalt fallback。还可继续补：

- `ios info display` 在 diagnostics relay 不可用或 iOS 17+ deprecated 时走 CoreDevice `getdisplayinfo`。
- lock state、完整 device info 的 CLI 暴露。
- 与 RSD service name 的 shim/remote 后缀兼容策略保持一致。

### P1：Tunnel CLI 与 tunnel manager 体验

HTTP manager 已有 `/`、`/tunnels`、`/tunnel/:udid` 等接口，但 CLI 顶层 `ios tunnel list` / `ios tunnel stop` 仍提示未实现，只能让用户手动调用 HTTP manager。对标 go-ios 的 `tunnel ls` / `tunnel stop`，这里应补成真实客户端行为。

### P2：XCTest / WDA

当前已经有最小启动路径，但参考项目覆盖更完整：

- XCTest result stream、test summary、失败/日志事件监听。
- 多 target/configuration 选择，而不是只取第一项。
- 更完整的 XCTestConfiguration 字段。
- iOS 版本分支，尤其旧版 testmanager / developer service path。
- WDA HTTP/native command client。

这部分需要真实 WDA/XCTest 环境验证；当前可以先做离线可测的解析、配置生成和事件模型，但不要宣称真实设备闭环完成。

### P3：恢复/刷机/低层设备生命周期

go-ios 和 pymobiledevice3 在 recovery/restore、固件、激活等低层生命周期能力上更完整。本项目已有部分 prepare/mobileactivation/imagemounter 能力，但如果目标仍聚焦 tunnel 和开发服务，这块优先级低。

## 推荐推进顺序

1. 补 fileservice 删除/移动等写操作，以及更完整的 data stream 协调。
2. 补 appservice 的 listroots/listapps/spawn/fetchicons/monitor，以及 launch options。
3. 接入 CoreDevice deviceinfo 到 `info display`、lock state 和完整 device info。
4. 补 `ios tunnel list` / `ios tunnel stop` 对本机 HTTP manager 的客户端调用。
5. 在没有真实 WDA 环境前，只补 XCTest/WDA 的离线可测部分，保留真实设备验证说明。

## 测试策略

- 离线测试优先覆盖 envelope、XPC value/plist 转换、错误解析、fileservice data frame、CLI 参数和输出格式。
- 真实设备测试必须覆盖 tunnel/RSD/fileservice/appservice/deviceinfo，因为这些协议细节经常随 iOS 版本变化。
- cargo 运行存在全局锁限制；并行分析可以交给子 Agent，但构建和测试应在主线程串行执行。
