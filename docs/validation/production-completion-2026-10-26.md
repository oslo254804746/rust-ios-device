# 剩余生产功能实施 — 2026-10-26

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

验证：Core Display23/0、orientation6/0；CLI完整1969/0/19ignored，含viewer28通过/2显式ignored；准确新制品`ios_cli-e7a77098f7474c2d.exe`显式HEVC2/0，包含持续解码与实际生产publisher在Web/VNC订阅者均不消费时只保留最新共享像素、旧Weak释放。日志`.git/completion-{cli-final,display-final,orientation,hevc-correct}.log`。旧二进制门禁结果作废，不混为本次新功能证据。

完整CLI首轮默认测试并发为1966通过/3失败/18ignored：旧方向501断言已随真实后端行为更新并增加非法方向无派发；两个未改动的tunnel时序fixture单独复跑通过，之后四线程完整批次通过。没有放宽期限/删除断言/忽略失败。
