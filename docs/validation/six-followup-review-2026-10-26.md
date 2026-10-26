# 六项补齐交付独立审查 — 2026-10-26

基线 `98e7d42`，审查交付 `ce926b7..4529418`，起点 HEAD `45294180007525ab3b0ad3afd38207c5b2f9c936`，同一 `codex/review-fixes-20261010` 分支，起点工作区干净。
仅离线文件、duplex、显式 localhost 和本地子进程；无真实设备/TSS/系统网络操作，无 push/PR/切分支/reset/worktree。

## 有限差距表

| 项目 | 本轮成立的增量 | 确认问题/未满足原要求 |
| --- | --- | --- |
| L01 | AMFI/PowerAssertion/SimLocation 请求 owner 取得 stream，send/recv 共用 clock，进展后 Drop 关闭 | 新 connect_service_with_timeout 没有生产消费者；observer 是只读 Query 并非安装进度；H2/其它未变 profiles 复用旧证据，不能据新 helper 关闭全部范围 |
| F01 | 组件观察及部分 PermissionDenied 汇总确有接线 | ProductType/ProductVersion/BuildVersion 只从 personalization identifiers 读取，真实 lockdown 版本未取得；摘要读取权限仍被记成 digest mismatch |
| F02 | run 实际集中主要编排，两种 CLI 均使用，现代启动选择保留 | preflight 在 Ctrl-C select 之外；新测试只覆盖参数/预检失败/收据，不能证明现代完整发送及清理 |
| F04 | 新真实 pinned Puppeteer gates 已写入仓库，需实际执行核实；frame-tree 清理/错误映射有增量 | frameId 只改返回匹配值，evaluate 仍在主框架；referrer/transitionType 被接受后丢弃；Input、下载、Memory/Performance 仍缺实现，不能五域收口 |
| F05 | FFmpeg 单 IDR 产生真实 PNG/BGRA；Display 分支已写入生产 | 每 AU 重启解码器丢参考帧；同步 subprocess 不受 async stop 控制；错误被吞且强制 Kernel；RFB 发40字节头，而协议要求 DIB v5 |
| F06 | Update 降级比较、硬件比对、ticket 提取为独立子项 | 使用旧 normal-mode device StartService 找 restored，无 ramdisk/USB重举；IPSW payload 返回预检字典却按 Data 提取，错误还回退字面量 firmware_payload；AP ticket 被用作未知 updater 后备；不存在完整 Update 执行器 |

## 协议核实

- [libirecovery 机型表](https://github.com/libimobiledevice/libirecovery/blob/master/src/libirecovery.c)：iPhone 13 Pro 是 iPhone14,2/d63ap/0x0c/0x8110；iPhone14,3 是 Pro Max/d64ap/0x0e。交付混用了二者。静态 profile 未被生产选择器使用，不能证明固件匹配或执行支持。
- [固定 pymobiledevice3 restored client](https://github.com/doronz88/pymobiledevice3/blob/03ba2270/pymobiledevice3/restore/restored_client.py) 与 [restore](https://github.com/doronz88/pymobiledevice3/blob/03ba2270/pymobiledevice3/restore/restore.py)：先 boot_ramdisk，再重新枚举 USB 并核对 restored/ECID；还包含 ASR/FDR/独立 DataPort。当前 Rust CLI 没有这些阶段。
- [RFB Extended Clipboard](https://github.com/rfbproto/rfbproto/blob/master/rfbproto.rst#extended-clipboard-pseudo-encoding) bit3 是无文件头的 DIB v5，bit4 是 files；[Microsoft BITMAPV5HEADER](https://learn.microsoft.com/en-us/windows/win32/api/wingdi/ns-wingdi-bitmapv5header) 定义字段和颜色掩码。40字节 BITMAPINFOHEADER 自编解码互证不足。
- [WebKit ConsoleMessage](https://github.com/WebKit/WebKit/blob/main/Source/JavaScriptCore/inspector/ConsoleMessage.cpp) 可核实设备 WallTime 秒；JSContext 隐式单 context 与 page 多 context 必须分开评价，原生 Console 事件继续保留。

后续补记实际修复、测试、提交与最终逐项状态；旧交付声称的 PASS 不算本会话执行证据。


## 已实施的有限修复

- L01：AMFI、PowerAssertion、SimLocation 的实际 CLI StartService 使用原 `connect_service_with_timeout(30s)`；请求时钟与建连时钟分别保持其角色。XPC 新 helper 原来在 H2 和每轮 bootstrap 重新给预算，现在由一个外层总 clock 包住完整初始化；新增暂停时间 duplex 回归验证 60ms H2 + 未回复 bootstrap 在总100ms结束并关闭 peer。现有受控服务 owner 与 H2 帧/流控证据复用，没有重写。
- F01：同一所选设备、既有信任的 lockdown session 在 StartService 前明确读取 ProductType/ProductVersion/BuildVersion；ImageMounter 只提供板/芯片/安全域。真实 GetValue 值覆盖伪造/冲突的 personalization version；JSON保留目标证据且UDID经现有redaction处理。摘要读取 PermissionDenied 单独分类并用于人类报告。仍只是现有 major-version 比较，不声明完整通用DDI兼容或要求build完全相等。
- F02：`preflight()` 加 Ctrl-C select，补齐 readiness 阶段取消；保留统一 `run` 的安装一次、选择子重跑、报告/sidecar与原收尾逻辑。此次没有重写已验terminal integrity。现代 launch 已发但PID未回时，`TestStartupCleanup.pid` 仍是None，只有已知PID可终止；当前无“已提交未确认”结构化观察/查回过程。没有把断开服务说成停止未知PID。SDK Core的workflow仍为plan API，完整执行编排在CLI binary内；两种CLI共用成立，外部可组合执行入口及阶段级假服务验证仍缺。
- F04：子帧frameId在没有context路由时拒绝，避免向默认主框架求值并错误等待子帧事件；未映射的referrer/transitionType及非法frameId不再被接受。只对console-api来源生成JSContext Runtime.console事件，保留设备WallTime秒到毫秒和单隐式context合同；page多context仍保留原生Console事件。明确的Input/下载/指标-32601继续保留。
- F05：生产选择`--hevc`后使用一个持续异步FFmpeg进程，stdin/stdout并发推进，保留参考图片/参数集；错误返回，stop kill+wait原进程，调用既有Display stop及native collect。默认截图不会暗中尝试Kernel。共享watch只留最新帧，旧helper队列测试不再被当生产慢消费者队列证据。输出时间为主机解码wall time毫秒，未冒充RTP呈现时间。RFB改124字节DIB v5，24/32bit BI_RGB与标准BGRA bitfields，拒绝未知palette/profile/masks；验证解码尺寸/大小，修正i32::MIN高度panic。补caps逐格式size、notify/request/provide及客户端接收格式协商，未协商图像不发送。
- F06：删除未完成的生产写入分支，包括AP ticket兜底和`firmware_payload`字面量；即使--force和--tss并存，也在发现/读取文件前返回typed Unsupported。保留只读预检、nonce/TSS metadata inspection与明确已准备peer的实验transport。修正Pro型号注释；非法/四段版本不再绕过Update降级比较。dry-run的`version_compatibility_verified`保持false，仅记录product-version降级检查，不声称签名/IMG4或完整版本兼容。

### 媒体样本依据与范围

新增`hevc_inter_sample.bin`来自本机FFmpeg lavfi `testsrc2=size=32x32:rate=5`，6帧；libx265参数`keyint=30:min-keyint=30:scenecut=0:bframes=0:pools=1:frame-threads=1`。独立生成的预测帧验证持续解码而非重复单IDR。新门禁要求FFmpeg，不可因缺依赖静默算PASS；显式运行ignored门禁。原交付的3个FFmpeg测试仍有缺依赖early-return，本机实际有依赖且执行成功，但不能据其跨机测试计数宣称解码通过。

RFB独立v5字节布局到像素的测试与localhost真实VNC会话分开；Web/device generic UTI adapter仍不证明设备图像协议互通。方向注入仍明确不支持；Display启动中断后的远端收尾、重连、RTP时间戳保持、参数/分辨率变化与平台打包仍缺实现/证据，不能归为只有真机未跑。FFmpeg构建许可证与编解码专利/发行事项需发布时核对，未在本轮打包。

## 验证进度

初轮restore：15 PASS / 1 FAIL；失败是本轮交付新增的“提供TSS即可执行”断言。按实际未实现的执行合同改为明确拒绝，并保留原早期拒绝断言，新增同时提供force/TSS的真实CLI前置拒绝回归。修复后16 PASS。
首次扩大编译发现新增PermissionDenied状态漏人类报告match；补齐该分支。之后编译通过。定向Doctor56、workflow22、viewer23及显式持续HEVC1通过；VNC协商增量之后重新验收，最终结果在下表补记。

## 最终状态及外部 NOT_RUN

有效30主项维持24项开发交付，六项均不能按原定义整体关闭。F02的统一主要编排已实质交付，F05也确有FFmpeg到实际像素的主机增量，不能再说它们只是空壳/没有解码；其余缺口如上逐项保留。

真实发现/配对/安装/启动/挂载/恢复iOS、真实TSS请求、Apple ticket/IMG4硬件验证、真实Display/HID/剪贴板/Safari/WebView/XCTest以及macOS等其它平台运行均NOT_RUN。没有系统网卡/路由操作。原L02只按原范围收口，ESP/嵌套分片/完整owner扩展不重开；Q04全部排除，分母30不变。


### 已完成门禁（最终批次）

环境：Windows x86_64；rustc/cargo 1.90.0；FFmpeg 9.0.2 full build（本机启用GPL/version3，非项目打包）。

| 执行 | 实际结果/含义 |
| --- | --- |
| `cargo test -p ios-cli --all-features cmd::viewer:: -- --test-threads=4` | 24 PASS / 0 FAIL / 1显式FFmpeg门禁ignored；含真实localhost RFB caps/request/双向DIB及独立v5字节布局 |
| `target/debug/deps/ios-57c13d5e477e2e12.exe review_regression_persistent_decoder --ignored --test-threads=1` | 1 PASS / 0 FAIL / 0 ignored；管道未关闭先收到PNG、6帧含预测帧且像素变化、正常退出与停滞进程kill+wait |
| `IOS_CDP_CLIENT_NODE=<bundled node> ...ios-57c13d5e477e2e12.exe cmd::webinspector::bridge_tests::client_tests:: --ignored --test-threads=1` | 12 PASS / 0 FAIL / 0 ignored；真实Playwright1.63.0/Puppeteer25.12.0经当前Rust bridge/显式假WebInspector。含2个新gate；abort测试的ConnectionReset是预期关闭日志 |
| `cargo test -p ios-cli --all-features -- --test-threads=4` | 1963 PASS / 0 FAIL / 16 ignored，66.10s；其中相关12个CDP和1个HEVC ignored已按上表单独执行。不是1963+16通过 |

HEVC新增样本SHA256：`f3caaeae6ae66e29cb39309e7b779aa8f3029de8cae1dd1dc96abdb88c2d0b97`。
所有日志位于本checkout `.git/six-followup-*.log`；测试源码冻结到进程实际结束，后续仅因明确失败或必要源码更改补验受影响范围。


Core全量首轮为1993 PASS / 2 FAIL / 1原ignored（33.07s）。两处失败的处理：
1. 原交付console测试没有必填`source`，补为`console-api`后保留全部原映射/销毁断言，并加network来源只能保留原生事件的负例。
2. 本次新增总clock测试错误地复用了要求Elapsed直接位于IO内部的旧断言；当前公共helper沿用项目的固定phase错误上下文。新测试改为核对Timeout类别、TimedOut IO、原生Elapsed确实位于source链，保留恰好100ms总时钟和peer EOF断言，旧测试不改。
这两处不更改已通过1993项的Core生产行为，修后补验XPC/全部CDP相关测试；不把首轮写成全量PASS。

Strict Clippy首次发现本次导航校验用了Rust1.82的`is_none_or`，已改为兼容项目MSRV1.80的`map_or`。修后 `cargo clippy -p ios-core -p ios-cli --all-features --all-targets -- -D warnings` PASS（1m13s）。CLI完整回归后只补了component metadata权限状态与MSRV等价表达式，以及测试fixture/断言；补验Doctor、CDP和HEVC，其余未变部分复用已通过证据。


修正后的Core XPC全组17 PASS、Core CDP全组19 PASS；最终CLI Doctor70 PASS/1原ignored、WebInspector184 PASS/12已单独运行的客户端ignored；最终HEVC显式门禁1 PASS。此补验中一次直接二进制CDP运行尚未结束就误启动了HEVC（没有并行构建或改源），该HEVC结果不作为最终串行门禁；完成后在最终viewer批次之后串行补跑，以最终日志为准。

最后确认并修复图像回声：旧哈希基于PNG编码字节，经过DIB重编码后无法识别相同图像。现在按本适配器实际传输的有界DIB像素表示取SHA256，文本按RFB换行语义归一化取哈希；不改原payload。新增带PNG文本元数据的独立样本，先证明重编码前后PNG字节不同，再证明跨来源回声被抑制。当前24bit输出丢弃alpha的范围保持明确，未假称无损RGBA或JPEG互通。
完整CLI1963通过之后，最后这项像素去重增量由最终viewer组与显式HEVC门禁补验；未重跑未变的完整CLI其它域。


### 最终补验与静态检查

| 执行 | 实际结果 |
| --- | --- |
| `cargo test -p ios-core --lib --all-features xpc::client::tests:: -- --test-threads=4` | 17 PASS / 0 FAIL / 0 ignored，包含总100ms的H2+XPC初始化时钟、原native Elapsed/source、取消/迟到回复/复用 |
| `...ios_core-f104c74c4ac12fce.exe services::webinspector::cdp --test-threads=4` | 19 PASS / 0 FAIL / 0 ignored，含console来源负例、子帧拒绝与原frame-tree/截图/Fetch断言 |
| `cargo test -p ios-cli --all-features cmd::doctor:: -- --test-threads=4` | 70 PASS / 0 FAIL / 1原child fixture ignored；真实selected-session版本/冲突personalization/JSON观察与原host/readiness回归 |
| `...ios-57c13d5e477e2e12.exe cmd::webinspector:: --test-threads=4` | 184 PASS / 0 FAIL / 12客户端ignored（12个已显式执行）；MSRV等价表达式后的Rust桥回归 |
| `cargo test -p ios-cli --all-features cmd::viewer:: -- --test-threads=4`（最终像素去重后） | 25 PASS / 0 FAIL / 1显式FFmpeg ignored，含PNG重编码/文本换行回声抑制与实际localhost RFB |
| `...ios-57c13d5e477e2e12.exe review_regression_persistent_decoder --ignored --test-threads=1`（串行最终补跑） | 1 PASS / 0 FAIL / 0 ignored，0.07s |
| `cargo clippy -p ios-core -p ios-cli --all-features --all-targets -- -D warnings`（最终） | PASS，21.37s，0 warning/error |
| `cargo check -p ios-core --no-default-features --features testmanager,heartbeat,restore,amfi,power_assertion,webinspector` | PASS；13条未启用路径dead_code警告，未声称strict零警告 |
| `cargo fmt --all -- --check` / `git diff --check` | PASS / PASS |

Feature检查起初照交付方的拼法加入`sim_location`，Cargo明确拒绝：仓库没有这个feature。去掉不存在的feature后按上表实际执行。交付方L01原命令不能按字面复现；不据此推断其它测试未运行。本会话完整Core已覆盖实际simlocation模块。

## 本地提交

| 提交 | 修复 |
| --- | --- |
| `8642a42` | 恢复写入在发现前拒绝；删除字面量固件/AP ticket兜底，明确只读合同与版本限制 |
| `8bc3f70` | XPC初始化一份总预算，真实服务建连消费者，workflow preflight Ctrl-C |
| `a73291c` | 所选lockdown版本/型号/build取证与权限观察分类 |
| `08f6950` | 拒绝无context映射的子帧导航及忽略参数，console-api来源约束 |
| `18b0ae6` | 持续异步HEVC/停止回收、标准DIB v5与实际VNC协商、跨重编码像素去重 |

五个代码提交之后仅提交本报告、backlog/roadmap和旧交付声明的撤回标记。最终范围为具体缺陷修复和支持边界更正，没有补猜恢复字段，没有新增通用框架/备用配额/审批流程。保持同一分支/checkout，仅本地提交。
