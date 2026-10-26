# 剩余六项独立复核与修复 — 2026-10-26

起点 `a703e81b5f5e77542179582d7dc8a18151fab65f`，分支 `codex/review-fixes-20261010`，工作区干净。
范围为 `b7c77a4..a703e81` 六包及交付文档；保留原 L02 收口和 Q04 排除。
本次只做源码、离线/duplex/localhost 验证和本地提交，无真实设备、TSS 或系统网络操作。
实际工具链：Windows x86_64，`rustc 1.90.0 (1159e78c4 2025-09-14)`、`cargo 1.90.0 (840b83a10 2025-07-30)`；未采用旧报告声称的1.92 nightly环境。

## 初始有限差距表

| 项目 | 生产代码核对 | 确认缺陷/有限修复目标 | 原主项状态 |
| --- | --- | --- | --- |
| L01 | 实际改动包含 diagnostics/screenshot/misagent/house_arrest/heartbeat，旧交付报告列错服务；前四者有 request owner 与总期限，CLI 有接线 | Heartbeat `send_polo` 借用 stream，调用方取消后部分写未失效；核对并修复该边界 | OPEN：本批不能证明所有原 profiles 关闭 |
| F01 | 新 target_manifest 被调用，但只传 UDID，board/chip/security-domain 全 None；None 被当通配符 | 禁止未知身份匹配成功，接入只读目标证据；修复组件过限仍汇总 Matched、缺字段等错误 | OPEN：本地匹配与平台/设备验收分别登记 |
| F02 | Engine 仅包装 CLI preflight/install，未拥有 launch/result/cleanup，也不在可复用 Core；readiness 吞错误 | readiness 不可由错误推导通过；CaseFinished/CaseFailed/SuiteFinished 绕过条目限额、长名称截断碰撞及无 incomplete 标记须修复 | OPEN：现代完整工作流仍缺实现 |
| F04 | 仅少量 reload/console 翻译和显式 Unsupported，客户端测试只改测试名称 | console 硬编码 context=1 与主机时间，未建立生命周期/原生时间关联；保留无依据域的明确拒绝 | OPEN：五域不得按旧十例测试总关闭 |
| F05 | HEVC 仅 NAL 解析，BoundedFrameQueue 未接生产；生产画面仍 PNG screenshot/RTP PNG；图像 UTI 路径缺证据 | 核查 RFB 扩展格式映射和图像声明，保留文本能力；明确 HEVC 解码/方向/设备图像取证欠账 | OPEN：三子项未全部交付 |
| F06 | CLI 把 normal-mode service 当 restored，provider 永远返回空固件；loop 把未服务 DataPort 计 served；缺 Status 默认0 | 关闭无证据 CLI 写入；dry-run 不得把 metadata correlation 称签名验证/执行支持；loop 拒绝未处理请求及缺终态字段 | OPEN：预检子项与实验传输不等于执行器；真实恢复待验收 |

后续在本文件记录修复提交、实际测试、仍缺实现与 NOT_RUN。旧报告的 COMPLETED/PASS 是待核对声明，不作为本次执行证据。

## 实际修复与支持边界

### L01

`DiagnosticsClient`、`ScreenshotClient`、`MisagentClient`、`HouseArrestClient` 的新request owner覆盖完整send/recv；有字节进展后Drop会关闭原stream。CLI diagnostics/info/mobilegestalt/batteryregistry/screenshot/provisioning/file/runtest使用有限预算。reboot/shutdown允许原EOF语义，House Arrest优先Status并成功交接原AFC stream；既有通过用例保留。

新发现并修复：Heartbeat的Polo写原来直接借用stream，部分写后取消仍可重用。现在先取得stream所有权，只有成功完整发送才返还。接收端原来没有对plist解码失败失效，可能重复读取同一个坏帧；现在除可恢复idle timeout外均失效。保留分帧接收状态和长空闲等待，未把监听改成短请求超时。

OPEN：不能据此关闭所有公开SDK兼容入口、CLI discovery/setup、HTTP2/startup/upgrade/blocking和清理profiles。兼容的无预算接口仍存在；原有证据可复用，但此次未补完整profile总验收。没有重开L02或全内存审计。

### F01

原调用仅构造UDID，board/chip为None时按通配匹配。现在使用同一显式设备、同一image mounter连接与总clock的既有`QueryPersonalizationIdentifiers(DeveloperDiskImage)`取得BoardId/ChipID/SecurityDomain。未知board/chip不能匹配成功，已知安全域不能接受缺字段identity；未观察安全域不能给出完整Matched。新增实际Source→session→service→local manifest fixture，验证更换板号会改变结果、组件digest命中且签名/可用性仍false。

修复manifest读取先限额、组件读取限额在真实读取中执行、超限摘要观察不再汇总Matched、空Manifest/缺路径不再宣称完整。主机usbmux计数继续仅为snapshot。

OPEN：当前版本字段观察/规则并不建立完整DDI版本兼容；本地文件读取不可中断的主机文件系统边界、权限分类和平台目录覆盖仍需独立验证。CRC/digest、personalization manifest和Apple签名验证不可混同。此次没有启用Developer Mode或挂载DDI。

### F02

原限额只接在SuiteStarted/CaseStarted，SuiteFinished/CaseFailed/CaseFinished仍可增长；丢弃事件不影响终态，超长名称还可截断碰撞。现在所有创建路径执行相同suite/per-suite/total限额，长标识用有界可读前缀+SHA256后缀保持区分；时间戳/未知状态/失败字段有界，日志截断提示占用原字节预算。结果丢弃或字段裁剪设置`results_truncated`，`terminal_outcome`拒绝CI通过，JUnit写入诊断；已有终态/sidecar规则保留。

readiness不再吞Developer Mode/LookupImage/CopyDevices/服务建连错误或把缺状态当已启用；保留原typed cause和deadline cause。显式安装仍只在首次阶段运行，重跑不重放安装；安装owner保留原实现。日志fixture改为显式离线connector，避免测试触发默认真实发现。

OPEN：`TestWorkflowEngine`事实上只有CLI preflight/install helper，launch/结果/清理仍在CLI中；Core的workflow模块是plan构建器。可组合完整Rust owner、现代CoreDevice/owned tunnel/RSD/remote DTX/AppService的端到端readiness/效果未知/清理流程尚未完整交付。signature存在只证明观察，不代表所有测试服务可用。旧terminal integrity通过证据不被重做或冒充现代完整流程。

### F04（按五域登记）

| 域 | 当前事实 | 未交付 |
| --- | --- | --- |
| Runtime/JSContext | 既有上下文生命周期/求值/关闭fixture保留；原生Console事件保留 | `ConsoleMessage`无context字段；移除硬编码context=1、主机当前时间生成的Runtime.consoleAPICalled；完整console映射待证据 |
| Page/iframe | 既有resource-tree/navigation/screenshot/reload子集 | 同源/跨源iframe的真实固定客户端流程及完整事件关联 |
| Input/截图 | PNG screenshot子集；Input明确拒绝 | 可信点击/键盘/修饰键/坐标/方向映射 |
| Fetch/下载 | 部分Network拦截适配；Browser/Page下载行为明确Unsupported | 完整继续/响应/失败/取消和下载客户端流程 |
| Memory/Performance | 明确Unsupported，未伪造零值 | 真实字段/单位/采样/生命周期映射及客户端流程 |

旧console测试原先断言伪造Runtime事件，本次用严格保留原生事件、禁止追加伪context/time的协议断言纠正；其余原断言未降低。Console依据：[WebKit Console协议](https://raw.githubusercontent.com/WebKit/WebKit/main/Source/JavaScriptCore/inspector/protocol/Console.json)。旧十例主要是JSContext/关闭/拒绝流程，其中默认Playwright用例的成功条件本来就是初始化拒绝；它们不是五域完整支持证据。

### F05（三步登记）

F05-1仍仅PNG像素解码/共享Web与VNC，HEVC仅NAL结构解析，BoundedFrameQueue为未接线helper。没有实现HEVC解码器、参数/关键帧变化和生产Display/RTP→decoder→Web/VNC路径；移除源码注释中无依据的许可证免责/完整媒体管线说法。

F05-2保留同一画面、输入与访问控制、断线清理；方向注入仍明确不支持。F05-3原PNG/JPEG在RFB位3/4传输错误；依[RFB规范](https://raw.githubusercontent.com/rfbproto/rfbproto/master/rfbproto.rst)，它们是DIB/files。本次生产只声明text，拒绝未实现格式，caps每格式一个size，重新SetEncodings可撤销扩展能力；扩展text校验长度、UTF-8、NUL和解压预算。旧私有image codec仅保留为原单元回归，已隔离生产，不能作为RFB互通证据。

HTTP/device已有PNG/JPEG通用UTI读写分支，但没有证明设备图像互通；Web mock与私有RFB压缩自测均不能替代设备协议证据。标准DIB转换、设备图像双向路径、orientation、生产HEVC/慢客户端媒体流程继续OPEN。

### F06

CLI旧非dry-run路径试图用normal-mode StartService连接restored，provider永远返回空固件，并把Digest当Ticket。已经删除该生产执行分支；`--force`仍在连接/发现设备前得到typed Unsupported。保留只读IPSW identity、组件CRC、nonce和saved TSS metadata inspection；签名验证、执行支持、版本兼容验证明确false。版本查询原来吞错并对同一key查询两次，现单次读取并保留原错误。

实验RestoredClient loop保留为已准备recovery peer的传输子项：缺失/非法Status不再默认为0成功；DataPort/Async/非已支持DataType和缺DataType明确失败，不计served；空firmware拒绝；provider/传输错误保留执行报告和原cause，按实际已写bytes计数；EOF不改称取消或成功。scripted Status=0只能验证本地状态机，不能证明恢复或IMG4验签。

OPEN（缺实现/协议证据）：明确机型/固件/Update路径计划；模式转换与ECID重新识别、recovery ramdisk启动、USB重举owner、nonce变化后的重新个性化/IMG4、所需ASR/FDR/data-port传输及其失败/清理。现有`verify_target_version`只校验非空字符串，文档已明确其不执行版本兼容策略，不能称目标版本精确匹配。`preflight-requests`已有identity选择和每updater新连接，但支持版本策略/完整目标证据仍须补齐。取证应针对同一明确machine/build，在另行授权的受控环境记录每阶段端点、消息与终态；本轮不实际取设备/TSS证据。

## 失败证据与验证规则

- 初次新回归在旧生产实现：4/4 FAIL，分别为`cancelled partial frame cannot be reused`、缺Status被识别成功、`unserved data request must not count as a successful update`、finish-only case数量超过上限。
- 首次测试代码有plist Dictionary key类型推断编译错误（E0283），补显式String类型后才获得上述运行失败；不算功能覆盖。
- 首轮扩大Core服务集：1116 PASS / 1 FAIL / 1原ignored。失败为原`test_recorder_bounds_and_truncation_markers`的名称保留断言；保留原断言，改为稳定摘要后缀和预算内提示后，Testmanager26/26通过。
- 首轮完整CLI失败与后续修复/最终门禁在下面补记。日志保留在本checkout `.git/remaining-six-*.log`，不把零筛选或ignored计PASS。

- 首轮完整CLI：1938 PASS / 8 FAIL / 13原ignored。8项均为旧Doctor假服务收到主机已有DDI目录触发的额外QueryPersonalizationIdentifiers（预期CopyDevices）。已为Source注入DDI目录：NativeSource保留实际目录，离线Source默认空目录；旧断言不变，Doctor重验69 PASS / 1原ignored。随后补本地identity/digest失败不能被远端manifest存在盖成绿灯的汇总，并由同一生产fixture断言。

- strict Clippy首次`--all-targets`发现原新增target_manifest测试7处cloned_ref_to_slice_refs；仅替换为`std::slice::from_ref`，保留所有原断言。重验strict Clippy通过。最后readiness调整后再检查CLI。
- readiness生产路径按实际读取版本选择既有查询：iOS16前不查询尚不存在的Developer Mode，17前观察Developer，17起观察Personalized。该有限协议profile不推导其它服务支持；增加实际duplex请求顺序fixture，避免强制缺字段失败破坏旧系统。原双image fixture断言保留。

## 最终门禁（串行）

| 命令 | 实际结果 |
| --- | --- |
| `cargo test -p ios-cli --all-features -- --test-threads=4` | 1947 PASS / 0 FAIL / 13原ignored；包含最终Doctor目录注入/本地失败汇总和legacy/Personalized readiness profile |
| `cargo test -p ios-core --lib --all-features -- --test-threads=4` | 1978 PASS / 0 FAIL / 1原ignored |

上述完整回归后仅收窄JUnit默认diagnostic分支到新增results_truncated，保持原unknown-status的根errors计数不重复；新增外部JUnit根计数断言并以testmanager集成补验，不重跑未变的全仓库。后续门禁与提交在本节补记。

最终18处Rust源码的逐文件SHA256清单在`.git/remaining-six-source-fingerprint.json`；排序紧凑JSON的SHA256为`bc74b26e1b523ec2888f8dfcadb06468aa4931a1506d522ecf4991d10b6bcbb6`。后续门禁期间源码冻结，提交前核对清单不变。

| 补验命令 | 实际结果 |
| --- | --- |
| `cargo test -p ios-core --all-features --test testmanager` | 19 PASS / 0 FAIL / 0 ignored；包含最后JUnit根计数断言及截断后的报告写入 |
| `IOS_CDP_CLIENT_NODE=<bundled node> cargo test -p ios-cli --all-features cmd::webinspector::bridge_tests::client_tests:: -- --ignored --test-threads=1` | 10 PASS / 0 FAIL / 0 ignored；实际固定playwright-core1.63.0 / puppeteer-core25.12.0，经真实Rust桥与显式假WebInspector；abort用例有预期ConnectionReset关闭日志 |

所有14个新增`review_regression`用例均在完整回归覆盖；旧私有image压缩自测不计图像互通，默认Playwright拒绝用例不计下载支持。

## 外部 NOT_RUN 与剩余实现

六项主项均OPEN，当前24/30开发交付口径不变。各项具体代码/协议缺口见上文，不能用外部条件替代。
本轮真实设备发现/配对/安装/启动/挂载/恢复、真实TSS与IMG4硬件验证、真实屏幕/输入/剪贴板、Safari/WebView与代表性XCTest应用全流程均NOT_RUN；其它平台实际运行也未新增验收。没有执行系统网卡/路由配置、push、PR、发布、reset、切分支或新worktree。

| 最终静态/feature门禁 | 实际结果 |
| --- | --- |
| `cargo clippy -p ios-core -p ios-cli --all-features --all-targets -- -D warnings` | PASS，最终JUnit/readiness源码，0 warning/error |
| `cargo check -p ios-core --no-default-features --features testmanager,heartbeat,restore` | PASS，13条未启用路径dead_code警告，未将其声称为strict零警告 |
| `cargo fmt --all -- --check` / `git diff --check` | PASS / PASS |
| 最终18文件源码SHA256核对 | PASS，门禁与六次本地提交期间无漂移 |

测试二进制SHA256：
- 最终CLI测试/Rust桥（`target/debug/deps/ios-57c13d5e477e2e12.exe`）：`c9d1d19efda599a99f9c36c860e8741ce37dd891b09b49a685eb5de4cc0ba2f1`。
- Core完整回归二进制（最后JUnit诊断收窄之前，随后用19项外部集成补验）：`dbfcb35ca04be337a5781ffe266ea443991af2c1167aeec82305f8496ae889e6`。

## 本地修复提交

| 项目 | 提交 | 修复结果 / 主项状态 |
| --- | --- | --- |
| L01 | `ac75d36` | Heartbeat取消写/坏帧失效已修；其它原profiles OPEN |
| F01 | `354e957` | 所选设备DDI身份/组件观察接通、失败不被盖绿；完整版本/平台证据 OPEN |
| F02 | `2491890` | 结果准入/截断与CI失败、readiness错误与版本profile已修；完整Rust owner/现代全流程 OPEN |
| F04 | `2f92f02` | 移除伪console context/time；五域完整客户端流程 OPEN |
| F05 | `fb53690` | RFB只声明已实现格式、拒绝错误图像线格式；HEVC/方向/设备图像 OPEN |
| F06 | `464daa9` | 拒绝空固件生产Update、修正签名/终态/served与错误报告；完整执行器 OPEN，真实恢复待验收 |

最终代码提交`464daa9`，后续只有本报告与路线图/当前账本的文档提交。保持同一checkout/分支，未push。
