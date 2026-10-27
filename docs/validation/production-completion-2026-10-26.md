# 六项生产功能继续审核与交付 — 2026-10-26～27

2026-10-27后续开发已交付：[开发增量与最终门禁](remaining-development-2026-10-27.md)。
当前有效30项为 **27项开发交付、F02/F04/F06三项仍未完成**；L01新增ZIP受控路径已收口。
下方26/30状态及制品计数属于前一审核批次，保留为历史证据；当前状态以上述增量为准。

## 前一审核批次（superseded by 后续开发增量）

当前代码基线：`c86308301a2ad9f61d7ab45d28d53435daced86d`。本轮实施起点
`9f0e78a86450ea5cacee7fff52dbad00ec5a63a1`；本次接手续审起点
`8f130054deffad237c9df7238e21e5a91c9ef942`。同一 checkout/分支
`codex/review-fixes-20261010`，仅本地提交；18个交接待提交文件全部承接。
主控负责修改、串行构建/测试/提交，三个 terra/xhigh 子代理只读审核不同代码包。
无真实 iOS 发现/配对/安装/启动/挂载/恢复、实际 TSS 或系统网络改动。

## 当前原定义与账本

有效30主项：**26项开发交付，4项未完成：L01/F02/F04/F06**。
本次仅 F01、F05 从原24/30计入开发交付，遵循用户“实现齐备，仅待外部运行可计
开发完成”的口径。没有将真实设备/平台 NOT_RUN 改成 PASS；Q01成熟度不升级。
L02原范围已收口，扩展 ESP/嵌套分片/完整 owner 审计不重开；Q04全部排除。
`a703e81`/`4529418` 的六项/30项全完成旧声明仍被撤回。

| 项目 | 原要求与生产交付 | 当前结果及确切剩余 |
| --- | --- | --- |
| L01 | 有限 setup/request/total/idle/cleanup profiles，实际 CLI 消费者、取消失效与原错误 | 进行中。安装/升级、GPX、共享setup、AppService和owned ZIP已接线并验证；ZIP兼容loader仍全量读入、同步解压且保留全部展开内容，缺该阻塞/每流缓冲边界的受控实现，不以总timeout包装当物理取消。 |
| F01 | 显式目标有期限只读 Doctor，主机/USB/remoted/目录/信任/Developer Mode/DDI/RSD/路由与稳定报告 | 开发交付。board/chip/security/model、generic personalized/legacy版本策略、本地组件/摘要/权限/部分超时与不完整证据均接线，Doctor78通过。真实设备、macOS remoted及未运行平台保持NOT_RUN。 |
| F02 | 可组合 Rust 执行入口与相同 CLI 编排，显式安装/启动/有限日志/结果/产物/清理 | 进行中。已有 TestWorkflowEngine::run 公开且两CLI共用；外部API1、实际XPC launch取消/重连/list/唯一新PID kill及DTX阶段已有证据。仍缺一个串起canonical/shim RSD、AppService与DTX的完整现代CLI假服务场景；代表性设备合成应用流程NOT_RUN。 |
| F04 | 在固定真实客户端/Rust桥上逐域实现 Runtime、Page/iframe、Input/截图、Fetch/下载、Memory/Performance | 进行中。normal context、iframe、Memory与Fetch子集已实现；15个真实固定客户端通过。页面多realm console关联、user/internal world映射、可信selected-page输入与下载控制/save/cancel/events仍OPEN。 |
| F05 | 真实解码Web、同源VNC/输入、方向、双向文本/图像剪贴板及有限重连/清理 | 开发交付。Viewer28、显式真实FFmpeg3、Core Display26/HID9通过；主机HTTP/RFB看见同一实际像素。物理Display/HID/图像互传、独立桌面VNC客户端和未运行平台NOT_RUN。stopAll服务级粒度为已声明限制。 |
| F06 | 目标/版本/identity预检和一种有依据的完整实验Update | 进行中。所选OS提取与真实ASR/SystemImageData/DataPort已接线；完整执行器仍缺recovery/ramdisk/USB-ECID重识别、nonce/TSS/IMG4/TBM/nonce-slot、FDR和完整异步dispatch。force/tss在发现前拒绝，真实恢复NOT_RUN。 |

## 本次确定修复

- 安装：owned ZIP一份clock覆盖准备/发送/flush/进度，每次IO拒绝迟到成功并drop原stream；零预算无文件或wire IO。apps通过同一CommandDevice/NativeScope持有现代连接。普通install/upgrade的300秒总期限现在包含暂存，取消/超时之后仍用新AFC连接在独立5秒内尝试temporary/final两路径清理，主因和清理cause均保留。修复外层select提前drop跳过cleanup；恢复同一选中设备上的modern→legacy Instruments回退。公开workflow安装helper的设备/服务/lookup共享30秒，随后owned ZIP总300秒。
- Doctor：个性化DDI最低iOS17检查独立于清单根ProductVersion；legacy仍要求major/minor。无效/未观察版本、缺失且无法排除匹配的identity字段和未知model约束均保持MissingFields。多清单优先保留相关硬件的具体部分观察，文件名排序不能被无关旧清单覆盖。目录/链接/非普通文件不能成为FileExists。摘要不支持/超时保留未知或部分状态，签名验证与设备可用性从不由本地文件匹配推导。
- CDP：fixture只发真实WebKit id/type/name/frameId；origin来自securityOrigin，auxData由适配器形成。导航/clear/detach与context ID共同退役旧查询，迟到成功、error、缺result及timeout均不能关闭或复活新realm。所有树查询捕获同一tree revision，跨导航快照不入缓存、不据此导航，而在原deadline内重查。未实现的user/internal worlds过滤；mapping失败使实际page/browser owner退出并清理inspector。
- Display/HID：MediaInUse9022不授权stopAll；只有重连成功并进入start调用才记录潜在提交。HID close使用strict stop，原EOF不能冒充停止ACK，释放HID与媒体共享close预算，确认关闭后仍幂等。所有启动/关闭路径保留原主因和清理cause；HidError::Media保留具体DisplayError source，迁移说明在lifecycle文档。
- Viewer：所有capture重试前收集原NativeScope，清理失败或本机decoder错误禁止重连；即使主操作成功、只有cleanup断线/超时，也不能重试。BGRA像素大小使用checked_mul。既有持续FFmpeg/MPEG-TS保留RTP PTS，Rust逐帧编码PNG，尺寸变化、慢消费者和真实HTTP/RFB像素断言保持。
- 工具/文档：能力清单工具跟随lib/bin共用入口寻找Commands，保留旧main布局fixture；Q01登记当前协议/主机证据并保持成熟度与real-device NOT_RUN。Viewer/CDP/restore/lifecycle/CLI README、CHANGELOG和三个当前账本同步；旧调度主表明确归档为superseded。

## 当前冻结源码的实际门禁

主控串行执行，编译期间未修改Rust源码。下面计数来自真实非零执行；先--list确认的显式门禁
没有把ignored或零匹配记成PASS。日志在本地`.git/completion-review-*.log`，不提交日志。

| 实际命令 | 结果 / 日志 |
| --- | --- |
| `cargo test -p ios-cli --lib --all-features --locked cmd::apps:: -- --test-threads=4` | 50 PASS/0 FAIL/0 ignored；0.17s，`completion-review-apps.log`。新3例实际AFC/InstallationProxy peers覆盖上传取消、提交Upgrade后deadline和独立cleanup失败。 |
| `cargo test -p ios-core --lib --all-features --locked -- --test-threads=4` | **2027 PASS/0 FAIL/1原ignored**；32.55s，`completion-review-core-full.log`。含ZIP24、Display26、HID9、CDP31、restore86。 |
| `cargo test -p ios-cli --all-features --locked -- --test-threads=4` | lib **1984 PASS/0 FAIL/21 ignored**；65.37s；外部workflow_api **1/0/0**。binary/doctest各0项，不计覆盖。`completion-review-cli-full.log`；含Doctor78、Viewer28、WebInspector184、Apps50。 |
| `target/debug/deps/ios_cli-e7a77098f7474c2d.exe cmd::webinspector::bridge_tests::client_tests:: --ignored --test-threads=1` | **15 PASS/0 FAIL/0 ignored**；34.81s，`completion-review-clients.log`。实际Playwright1.63.0/Puppeteer25.12.0，不升级依赖。 |
| `target/debug/deps/ios_cli-e7a77098f7474c2d.exe cmd::viewer:: --ignored --test-threads=1` | **3 PASS/0 FAIL/0 ignored**；0.18s，`completion-review-media.log`。真实持续HEVC、RTP回绕/不等间隔/32x32→48x32共9帧、HTTP/RFB及慢消费者。 |
| `cargo clippy -p ios-core -p ios-cli --all-features --all-targets --locked -- -D warnings` | PASS，21.00s，`completion-review-clippy-final.log`；首轮61s也通过，最后两项小修后按最终源码复验。 |
| `cargo check -p ios-core --no-default-features --features testmanager,heartbeat,restore,amfi,power_assertion,webinspector --locked` | PASS；原13条dead_code warning，未声称strict，`completion-review-minimal.log`。 |
| `cargo build -p ios-cli --all-features --locked` | PASS，1m19s，`completion-review-build.log`。 |
| `cargo fmt --all -- --check` / `git diff --check` | PASS / PASS。 |
| `python -m unittest discover -s scripts -p test_capabilities.py -v` | 6 PASS/0 FAIL；lib/main双布局及证据策略，`completion-review-capability-tests.log`。 |
| `python scripts/capabilities.py --binary target/debug/ios.exe` | PASS：119 records，actual CLI --help一致；`completion-review-capabilities.log`。最终登记后策略测试再验6/0，`completion-review-capability-tests-final.log`。 |

CLI的21 ignored中15客户端和3媒体已按上表显式运行；其余为原Doctor child fixture、S02 Chromium gate和L04测量入口，本次不计PASS。Core原ignored也不计PASS。
当前Runtime tests里的预期ConnectionReset日志来自被取消的客户端fixture，不是额外失败。

制品与源码对应：当前所有Rust/JS代码已在上述基线提交，后续仅文档/登记变动。

| 制品 | SHA256 |
| --- | --- |
| `target/debug/deps/ios_cli-e7a77098f7474c2d.exe` | `EF739967B489E80E83575E7FF15CD9D7E8A2E07D7B8AFB2E10C23EF26F936E3F` |
| `target/debug/deps/ios_core-f104c74c4ac12fce.exe` | `9C58158D867B4E565FBCC9AF53ADB35094522D6F234A62AA21883D5B1F5D230B` |
| `target/debug/ios.exe` | `CC75763F25D7A2DDA788F4137BC2A1EF7C5E98EFEC7D41EDD6B153A5F1ECF06E` |

本机PATH Node为24.19.0，FFmpeg为9.0.2-full_build（GPL/version3，外部依赖、不打包）。
最早误选`ios_cli-30048c2042a0cc75.exe`的旧结果无效，本报告只使用上述Cargo实际产出的制品。

## 协议依据、保留的限制与外部验收

- DDI硬件选择、版本下限、Display stop、Orientation和pasteboard字段来自固定[pymobiledevice3 03ba2270](https://github.com/doronz88/pymobiledevice3/tree/03ba2270)。个性化清单/本地SHA256/SHA384并非Apple信任链验证。legacy raw DTX仅已实现TLS1.2，TLS1.3明确Unsupported；不借合成握手扩大范围。
- WebKit固定[Runtime.json](https://github.com/WebKit/WebKit/blob/c1abbcfd045014c75f90751dad1a3e489e25ed47/Source/JavaScriptCore/inspector/protocol/Runtime.json)、[Memory.json](https://github.com/WebKit/WebKit/blob/c1abbcfd045014c75f90751dad1a3e489e25ed47/Source/JavaScriptCore/inspector/protocol/Memory.json)、[Network.json](https://github.com/WebKit/WebKit/blob/c1abbcfd045014c75f90751dad1a3e489e25ed47/Source/JavaScriptCore/inspector/protocol/Network.json)及对应agent实现支持normal context、采样bytes和拦截生命周期。Browser.json存在，但无已验证下载控制语义。JSC单隐式realm console可转换设备WallTime；页面多realm不伪造context=1/time=now。
- Memory的Timestamp是设备inspector elapsed秒；WebKitJavaScriptMemory含GCHeap+GCOwned，不能冒充Chrome JSHeapUsedSize/TotalSize，缺分类不补零。Fetch支持wildcard request/response stage、override/fulfill/fail及pending disable；无对应authentication/resource filter仍拒绝。可信Input不能用JS dispatchEvent或全局HID空成功替代。
- [RFC7798](https://www.rfc-editor.org/rfc/rfc7798.html#section-4.1)及固定[FFmpeg n7.1 mpegtsenc](https://github.com/FFmpeg/FFmpeg/blob/n7.1/libavformat/mpegtsenc.c)/[showinfo](https://github.com/FFmpeg/FFmpeg/blob/n7.1/libavfilter/vf_showinfo.c)用于RTP/PTS依据。方向等待真实后端；PNG alpha用32bit BI_BITFIELDS，JPEG RGB/灰度转DIB，CMYK/16bit拒绝。Standalone viewer clipboard仍text-only。外部FFmpeg运行方案与打包影响见[viewer](../viewer.md)。
- ASR按固定[asr.py](https://github.com/doronz88/pymobiledevice3/blob/03ba2270/pymobiledevice3/restore/asr.py)实现无长度头XML Initiate/offer、OOB与128KiB payload及协商SHA1；选中OS提取64KiB分块/CRC/SHA256。两者均不能声明flash完成。参考iPhone14,2/d63ap（13 Pro），board0x0c/chip0x8110，iOS17.5.1/21F90；iPhone14,3/d64ap（Pro Max）不可替代。用户已确认暂无该离线IPSW，不再重复询问，也不把缺文件当完整执行器唯一障碍。

真实USB/Wi-Fi、配对/Developer Mode/DDI挂载、macOS remoted前后、Safari/WebView、
合成测试应用在代表性设备的完整workflow、物理Display/HID/PNG/JPEG剪贴板、独立
桌面VNC及其它平台执行、真实TSS/硬件恢复均NOT_RUN。源码/协议缺口在当前表单独列出。
后续代表性设备步骤沿用显式UDID、只读doctor与ready检查、显式安装包、runtest/runxctest
同一engine、有限日志/结果/JUnit/sidecar及cleanup，分别核对成功/失败/取消/重跑；本轮不执行。

## 本地提交映射

| 提交 | 工作包 |
| --- | --- |
| `01f9640` | DDI版本策略、目录/权限观察，建立本轮报告 |
| `3219ebd` | InstallationProxy有限安装/升级，GPX逐点写与取消 |
| `7e1ef54` | 公开现有workflow执行库入口、未确认launch基础 |
| `ea7fa65` | 实际ASR与RestoredClient SystemImageData接线 |
| `6bc36d5` | Doctor部分deadline观察、外部workflow API测试 |
| `c74bad3` | retained Display、方向、RGBA/JPEG剪贴板 |
| `751ecea` | iframe/native Memory/Fetch生产生命周期与固定客户端 |
| `a9e0647` | 实际CLI消费者共享setup总clock |
| `787fc0e` | 真实XPC launch取消→重连→list→唯一新PID kill fixture |
| `9d4ec02` | RTP PTS→MPEG-TS→持续FFmpeg BGRA/Rust PNG与尺寸门禁 |
| `2bfa790` | 所选IPSW OS组件提取 |
| `23ad537` | CDP rollback保留原错误和cleanup观察 |
| `8f13005` | AppService有限请求、CLI共享setup |
| `e244013` | 本次整合ownedZIP/apps NativeScope；修复暂存取消/超时cleanup与Instruments回退；public安装helper setup/lookup clock |
| `9f395ac` | Display提交意图与cleanup cause；strict且幂等HID close、原生EOF/预启动重连失败fixture |
| `6de6b9d` | native context schema/退役/树revision，实际owner退出与客户端fixture |
| `f717738` | generic personalized最低版本及完整/部分DDI证据分类 |
| `4cca4af` | media重试前原生收尾、cleanup-only失败禁止重试与尺寸溢出 |
| `c863083` | Q01工具识别公开lib/bin入口；6策略测试 |

此前历史fixture失败均在原审查/旧日志中保留：PNG直出在尺寸变化坏IHDR后改为BGRA；
iframe先等匹配commit；RFB允许一条合法异步Notify后仍严格核对Provide/DIB；原tunnel
高并发时序失败在未改期限/断言的4线程整批复验通过。本次冻结源码完整门禁无失败。
没有删除/ignore/弱化旧断言或提高旧测试期限，未回滚交接改动；不把文档登记当新设备支持。
