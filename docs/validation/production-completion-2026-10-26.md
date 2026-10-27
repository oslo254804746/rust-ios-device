# 剩余生产功能实施 — 2026-10-26～27

起点 `9f0e78a`，分支 `codex/review-fixes-20261010`，工作区干净。同一 checkout，仅本地提交。主控串行构建，所有设备协议验证使用离线/duplex/localhost；无真实设备、TSS或系统网络修改。

## 有限实施表

| 项目 | 本轮具体工作范围 | 验收出口 |
| --- | --- | --- |
| L01 | 实际 InstallationProxy 安装/升级进度 owner，GPX 请求与取消；复用已有 H2/DTX/AFC/Lockdown 等 profiles | 实际请求、部分发送取消、迟到消息/不可复用、主错误及清理 |
| F01 | 个性化/传统 DDI 版本策略、型号约束、目录错误与不完整报告 | 硬件/版本/组件/目录 fixture、无设备报告 |
| F02 | 现代启动已发送未确认 PID 的观察/收尾，公开可组合执行入口 | canonical/shim/DTX/AppService 假服务阶段及取消 |
| F04 | 根据 WebKit 实际协议推进上下文、输入、Fetch、指标 | 固定实际客户端经过 Rust 桥，无伪可信输入或常量指标 |
| F05 | Display 启停 owner、RGBA DIB、时间戳及生产慢消费者 | 原服务/任务收尾、真实像素与标准 RFB 流程 |
| F06 | 固定上游恢复路径逐部件核实并实现可独立执行部分 | 离线 IPSW、假 TSS/恢复服务；未实现的端点不得接通 CLI |

## F01 首包

个性化 DDI 不要求与设备同一 OS major/build，而按 board/chip/security identity 匹配；iOS17 之前不能选择此 profile。传统镜像检查 major/minor；若清单提供 SupportedProductTypes，额外检查目标 ProductType。目录枚举、entry/file_type 错误不再吞成 NoManifestFound；组件 metadata 的 PermissionDenied 不再被 is_file 提前吞掉。存在/摘要匹配仍不代表 Apple 签名已验证或设备已挂载。

协议来源：[固定 pymobiledevice3 mobile_image_mounter.py](https://github.com/doronz88/pymobiledevice3/blob/03ba2270/pymobiledevice3/services/mobile_image_mounter.py)，`request_personalization_manifest` 按 BoardId/ChipID 选择，不比较设备 build；`PERSONALIZED_IMAGE_MIN_VERSION` 为17。

执行 `cargo test -p ios-cli --all-features cmd::doctor:: -- --test-threads=4`：72 PASS / 0 FAIL / 1 原 child fixture ignored。日志 `.git/completion-doctor.log`。六主项计数暂不调整，后续包完成后统一登记。

## L01 安装与 GPX 生产消费者

新增 InstallationProxy `install_with_timeout`/`upgrade_with_timeout`，请求与所有进度回复共享一个总 clock。实际 apps install/upgrade 使用30秒服务建连、300秒操作总期限与 Ctrl-C；停止不声明设备安装已回滚，已发送后效果可能未知。部分发送/等待取消使原连接不可复用，迟到 Complete 不能被下一操作消费。暂存路径清理独立5秒，原操作和原清理错误均保留。

GPX CLI 现在使用 SimLocationClient 的逐点写入 owner（30秒请求），计划轨迹等待属于可取消 idle，Ctrl-C 关闭原客户端；此前已发送的位置可能仍有效。保留旧 raw API 的兼容行为。

Core installation30/0、simlocation11/0；CLI apps46/0、location定向通过，日志 `.git/completion-{install,gpx,apps-cli,location-cli}.log`。首次新fixture编译类型歧义已修，不作通过证据。既有H2/DTX/AFC等未变profiles复用交接证据；未凭这两个消费者声明全仓库L01完成。

## F02 执行入口与未确认启动

`ios-cli` 现在同时提供库，公开现有 TestWorkflowEngine/Args/RunOptions；binary仅调用同一run_cli。没有复制编排引擎，runtest/runxctest仍调用原run。XPC `call_observed` 在完整请求本地flush后记录submission；CoreDevice launch把此观察与已确认PID分开。现代启动在launch前保存同bundle旧PID基线，若已完整发送但PID未确认，cleanup在原服务reconnector上查询新进程，只对唯一、同bundle且不在旧基线的PID发送kill。未能查回不把关socket当成功，返回RunnerLaunchUnconfirmed或原重连/查询错误，继续testmanager/native收尾。

验证：runtest12/0，统一engine22/0，runxctest10/0，XPC18/0（含真实duplex完整发送后caller Drop、不可复用及peer EOF）。日志 `.git/completion-{workflow,engine,runxctest,xpc}.log`。公开API外部集成测试已注册，最终批次执行。尚未用这些结果声称现代canonical/shim/DTX/AppService全阶段假服务已经全验收。

## F06 ASR 数据端口

新增 `restore::asr::send_image`，实现固定上游的无长度头XML Initiate/offer、OOBData随机范围读取、Payload切换、128KiB分块和协商后每块SHA-1。复用调用方可seek镜像，仅保留一个块，不把ZIP预检字典或AP ticket替换成固件。`RestoredClient::send_system_image`实际解析SystemImageData/DataPort，通过调用方提供的同设备端口连接器执行；与原restored操作共用总deadline/cancel。开始服务该请求即将控制owner标为不可复用，完成后才恢复；caller Drop或失败不能继续接收迟到消息。ASR收据只描述本地发送，不能推进恢复到Completed。

来源：[固定ASR实现](https://github.com/doronz88/pymobiledevice3/blob/03ba2270/pymobiledevice3/restore/asr.py)、[send_filesystem](https://github.com/doronz88/pymobiledevice3/blob/03ba2270/pymobiledevice3/restore/restore.py)。

Core restore85/0（新增4个ASR wire与1个restored实际DataPort接线测试，包含OOB溢出、校验、timeout原Elapsed、取消/EOF）；`.git/completion-asr-final.log`。首轮泛型重名编译失败已修，不算通过。原未服务SystemImageData的旧恢复loop拒绝断言保留，新增API不声称该旧loop已支持所有data类型。没有接通CLI完整Update；USB recovery/ECID重新枚举、各组件IMG4/TBM/nonce-slot、FDR与完整firmware dispatch仍需实现。

## F01 部分本地观察

组件摘要读取增加同一probe deadline的逐块检查，超时保存已完成组件及未完成组件状态，不伪装为digest mismatch。其它读取失败、未知摘要算法和超过读入上限都表示read_failed/摘要未知。同步OS文件读取仍是协作检查，不能抢占卡在内核中的单次read。CLI人类报告与JSON均区分这些状态。当前CLI全量包含Doctor73通过、原child fixture忽略。

## F05 媒体/剪贴板/方向

Display新增retained startup，caller取消仍保留原服务及reconnect factory，主机验证实际start提交→取消→原factory新控制连接→严格stop请求/真实回复→两peer EOF。Viewer/HID启动owner接线，视频退出要求真实stop回复，不再把EOF当设备停止ACK。方向接入已有OrientationServiceClient并等待其实际结果；HTTP成功前必须完成后端请求，非法方向不派发。

来源：[固定OrientationService](https://github.com/doronz88/pymobiledevice3/blob/03ba2270/pymobiledevice3/remote/core_device/orientation_service.py) 的Xcode sniff字段；[固定DisplayService](https://github.com/doronz88/pymobiledevice3/blob/03ba2270/pymobiledevice3/remote/core_device/display_service.py)的StopRequest。stopAll仍是服务级停止，未虚构UUID为stream token。

有alpha的PNG现在输出32位BI_BITFIELDS DIB v5，保留RGBA；RGB保留原24位输出。JPEG通过固定jpeg-decoder0.3.2（MSRV1.61，MIT/Apache2，无默认rayon）解码RGB/灰度再转DIB，CMYK/16位明确拒绝。JPEG和PNG按实际传输像素去重。样本由本机FFmpeg lavfi红色16x16生成，SHA256 `ad92fb5cf7f4fcdb500c5f0a6e03c8117fd6c44fc013dc4087f3d7597c5f2255`。

验证：Core Display23/0、orientation6/0；CLI完整1969/0/19ignored，含viewer26通过/2显式ignored；准确新制品`ios_cli-e7a77098f7474c2d.exe`显式HEVC2/0，包含持续解码与实际生产publisher在Web/VNC订阅者均不消费时只保留最新共享像素、旧Weak释放。日志`.git/completion-{cli-final,display-final,orientation,hevc-correct}.log`。旧二进制门禁结果作废，不混为本次新功能证据。

完整CLI首轮默认测试并发为1966通过/3失败/18ignored：旧方向501断言已随真实后端行为更新并增加非法方向无派发；两个未改动的tunnel时序fixture单独复跑通过，之后四线程完整批次通过。没有放宽期限/删除断言/忽略失败。

## F04 iframe、内存采样与Fetch

Page适配器保存WebKit原生`ExecutionContextDescription.id/frameId/type=normal`，只对frame-tree内且存在正常world映射的子帧求值导航；没有映射继续拒绝，明确未知context不转向主框架。真实Puppeteer分别选择同源与跨源子帧context、求值、导航并等待对应原生commit。

Performance.enable/disable实际调用Memory.enable/startTracking/stopTracking/disable，getMetrics只返回实际采样：Timestamp为设备inspector elapsed秒，WebKitJavaScriptMemory/JIT/Image/Layer/Page/OtherMemory为原生bytes，缺失分类不补零。JavaScript分类包括GCHeap+GCOwned，未冒称Chrome JSHeapUsedSize/TotalSize。HeapProfiler.collectGarbage映射Heap.gc；DOM/listener计数等无对应仍拒绝。启动采样失败会disable并保留原错误。WebKit断开frontend时亦执行disable移除observer。

Fetch增加native interceptWithRequest（url/method/headers/base64 postData）、响应阶段interceptWithResponse；fail保留Cancellation等实际映射。disable先由native关闭拦截、继续pending请求，再removeInterception精确原stage/regex，重启用不残留旧pattern。enable失败移除本次filter，原错误保留；pending状态禁止重叠更改。假服务不把客户端CDP消息直回，仍经现有Rust桥和WebInspector plist。

来源：[WebKit Runtime协议](https://github.com/WebKit/WebKit/blob/c1abbcfd045014c75f90751dad1a3e489e25ed47/Source/JavaScriptCore/inspector/protocol/Runtime.json)、[Memory协议](https://github.com/WebKit/WebKit/blob/c1abbcfd045014c75f90751dad1a3e489e25ed47/Source/JavaScriptCore/inspector/protocol/Memory.json)、[InspectorMemoryAgent](https://github.com/WebKit/WebKit/blob/c1abbcfd045014c75f90751dad1a3e489e25ed47/Source/WebCore/inspector/agents/InspectorMemoryAgent.cpp)、[Network协议](https://github.com/WebKit/WebKit/blob/c1abbcfd045014c75f90751dad1a3e489e25ed47/Source/JavaScriptCore/inspector/protocol/Network.json)、[InspectorNetworkAgent](https://github.com/WebKit/WebKit/blob/c1abbcfd045014c75f90751dad1a3e489e25ed47/Source/WebCore/inspector/agents/InspectorNetworkAgent.cpp)。

执行：Core CDP22/0（之后Fetch移除增量新增fixture在最终批次补验），CLI WebInspector184/0/15客户端ignored；准确当前制品显式客户端15/0/0ignored（Playwright1.63.0/Puppeteer25.12.0，包含新iframe/metrics/Fetch三个流程）。`.git/completion-{cdp-core,web-final,cdp-clients-all}.log`。首次新iframe gate13/1失败，原因为测试把navigate回复当作异步事件也已送达；保留全部断言并显式等待匹配事件，随后单项和最终15全组通过。Input/下载和不等价的Chrome专有计数未实现，不能由这些流程宣称五域完成。

## 后续生产增量与最终批次

- L01：新connect_lockdown_service实际用于Diagnostics、Misagent、SpringBoard、Screenshot、Heartbeat、AMFI、PowerAssertion、SimLocation和AFC/HouseArrest。30秒setup总clock覆盖所选设备连接/lockdown/StartService/端口/TLS；请求另用其原总clock，长流不继承setup期限。Heartbeat等待显式Ctrl-C可取消。setup两项fixture验证零预算不poll、60ms发送后仍在总100ms结束、caller Drop及原peer EOF。没有将该时钟fixture冒称完整假USB/TLS握手。
- F02：启动前PID基线在缺少listprocesses能力时允许Unknown，已确认PID流程继续工作；部分提交后失效也进入未确认收尾。两个新实际XPC假服务测试经过生产TestStartupCleanup：完整launch提交后取消→原factory重连→listprocesses→只kill唯一新PID→原peer EOF；歧义时不发送猜测PID的kill。原错误加RunnerLaunchUnconfirmed上下文，不跳过其它服务/native收尾。首轮fixture将原生Int64 PID误写成Uint64断言，修正后runtest14/0。
- F05：用最小单HEVC轨MPEG-TS携带90kHz RTP PTS，经持续FFmpeg解码，读取showinfo提供的对应解码帧PTS和尺寸；BGRA原始帧由Rust逐帧编码PNG。避免FFmpeg PNG编码器在分辨率变化时保留旧IHDR造成坏帧。Web以X-Timestamp-Clock区分rtp-relative-ms/host-unix-ms；逐帧sequence允许重连后RTP原点重置。已实现有限次数、可配置退避重连；原清理失败或本机解码器失败不重启设备服务。只读剪贴板GET失败可刷新服务一次，SET/HID不重放，图像通道上限修正为2MiB，失去媒体帧时禁止沿用旧输入尺寸。
- F06：新增选中identity的OS镜像提取，固定64KiB缓冲、大小/ZIP CRC/SHA256、协作总clock与取消；失败输出是调用方必须丢弃的部分文件。已由Core全量覆盖成功和首块后取消。
- F04：原生Memory/Fetch rollback自身失败或超时仍保留原命令错误，并在error.data保留cleanup观察；24个Core CDP测试通过。

RTP/传输来源：[RFC7798§4.1](https://www.rfc-editor.org/rfc/rfc7798.html#section-4.1)、[FFmpeg n7.1 mpegtsenc](https://github.com/FFmpeg/FFmpeg/blob/n7.1/libavformat/mpegtsenc.c)、[showinfo](https://github.com/FFmpeg/FFmpeg/blob/n7.1/libavfilter/vf_showinfo.c)、[rawvideo逐帧尺寸](https://github.com/FFmpeg/FFmpeg/blob/n7.1/libavcodec/rawenc.c)。镜像UTI和XPC数据表示来源：[固定pasteboard service协议观察](https://github.com/doronz88/pymobiledevice3/blob/03ba2270/pymobiledevice3/remote/core_device/pasteboard_service.py)、[Apple图像UTI](https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/CocoaDrawingGuide/Images/Images.html)。这些是协议依据，不是本机物理iOS互通的运行记录。

时间戳门禁真实执行9帧、32位RTP回绕、不等间隔、32x32→48x32参数切换，并经真实HTTP/RFB端点取得同一PNG/像素；Web/VNC订阅者停滞时旧共享像素Weak释放。原始RTP API未改。首轮PNG直出在第7帧坏IHDR失败，未删尺寸断言；改为rawvideo/Rust PNG后通过。

| 执行 | 结果 |
| --- | --- |
| Core lib all-features，4线程 | 2011 PASS / 0 FAIL / 1原ignored，32.19s；`.git/completion-core-final.log` |
| CLI实际当前lib测试二进制，4线程 | 1975 PASS / 0 FAIL / 21 ignored，66.14s；`.git/completion-cli-verified.log` |
| 显式当前媒体门禁 | 3/0/0ignored；`.git/completion-media-final.log`，包含实际HTTP/RFB和慢订阅者 |
| 固定实际客户端 | 15/0/0ignored，35.20s；`.git/completion-client-verified.log` |
| 后续Core CDP补验 | 24/0；`.git/completion-cdp-core-final.log` |
| 后续Viewer补验 | 28/0/3显式媒体ignored；`.git/completion-viewer-final.log` |
| strict Core+CLI allfeatures/alltargets Clippy | PASS，54.09s；`.git/completion-clippy-final.log` |

CLI集成有一次1974/1/21ignored：合法异步DIB Notify抢在Provide回复前到达，旧fixture假定首条必是Provide。修后精确验证可选的单条Notify，继续保留原Provide/DIB/双向像素断言；未增超时，最终1975全量和Viewer门禁通过。Core完整批次之后的唯一Core生产增量是rollback错误保留，已由最终CDP24项和strict检查补验。

用户于2026-10-27确认暂时没有所选21F90离线IPSW；完整Update仍缺USB recovery/ECID重新枚举、IMG4/TBM/nonce-slot、FDR及全数据请求编排。没有把ASR/提取或任意Status=0记成完整恢复。真实设备/TSS/平台验收保持NOT_RUN。

### AppService与共享setup补包

Core AppService新增可选有限请求总期限（默认None保留兼容），单次查询/kill/launch/spawn及完整streamapplist收集使用同一发送/接收总clock。真实apps CLI设30秒请求与XPC建连期限；process monitoring及公开stream APIs保留调用方idle/lifetime合同，不被30秒误杀。观察型launch同样受预算控制，保留完整发送事实；超时后client不能消费迟到PID。默认设备发现和共享lockdown/userspace/版本探测/版本选择建连也纳入总setup期限。

Core AppService26/0，实际duplex验证零预算不发字节、提交后100ms超时/失效、streamapplist总期限、monitor不继承短请求预算；CLI apps46/0。最小feature检查PASS，13条原dead_code警告；strict Core+CLI allfeatures/alltargets PASS。日志`.git/completion-{appservice-deadlines,apps-service-final,minimal,clippy-service-final}.log`。新fixture首轮误用通用source遍历寻找stdio内部Elapsed，已改为核对原XPC IO边界、TimedOut类别和IO.get_ref中的实际Elapsed，未放宽时钟/失效/发送断言。
