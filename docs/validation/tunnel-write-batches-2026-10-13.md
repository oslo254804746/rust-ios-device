# L02 outcome/receipt/displaced批次字节准入 · 2026-10-13

起点 `700c271b7837cee26e83f79fed18c6874847624c`，分支/check-out保留、工作树干净；
540个构建来源文件和CLI SHA256重新核对一致。默认unit Source仍为NoDevices，
生产NativeSource及原localhost native/source fixtures保持，未进行真实发现/设备操作。

## 生产实现

Source window开始前，WriteBatch按候选数预分配并准入三个实际Vec backing：
outcome intents、原Source receipt引用及displaced失败row。每个候选最多一条outcome/
receipt/移出的row；已有4096上限继续，Source后不会再为这三个集合增长或争额度。
准入失败先释放已获charge，不创建Source/native owner，不制造backoff或ready。

manual-success批次同样准入，completion版本列表使用KnownVec。原native/Source
错误和Input owner仍为原对象；SourceFailuresContext直接持已准入receipt Vec与charge，
保留primary/secondary、downcast及原地址，不格式化原任意错误。caller保留报告时，
manager clear/Drop不能提前退款；最后引用释放才释放对应backing。

共享receipt owner登记在原scan Session上，覆盖连接window、整体clock、manual
success收尾、failure writer、发现原错误合并及readiness。内部deadline/cancel marker
由外层转为实际timer/epoch结果后，Session才完成总报告，避免丢失已经取得的Source
错误。及时写入保持Recorded，phase失败保持Failed，未完成caller Drop标Abandoned；
前缀Recorded不会被随后阶段覆盖。短同步mutex内只取得/移动引用，不做网络等待。

## 验证与边界

新增7项门禁通过：三份backing精确额度/一字节拒绝/完整退款、count拒绝、Source
之前拒绝无connect/receipt/native/backoff/ready、manual cleanup真实Elapsed+原IO地址、
整体marker转换后的原IO、writer完成后ready超时保持Recorded，以及caller Drop真实
Abandoned。这些行为经过实际localhost TCP refusal和生产scan/Session/flusher接线。

初次编译fixture对ScopeDenied调用unwrap要求Debug，改用明确的测试断言，未改变
生产权限类型。首轮5通过/1失败用250ms总期限，Windows localhost原IO未在该期限
取得，不能冒充错误保留负控制；改用3s实际总期限并在已到达的manual收尾阻塞
3.5s，明确断言原IO已经取得、最终为真实Elapsed。readonly source/profile等原断言
不放宽，原失败日志保留。另加ready-after-write用例验证共享owner不提前结束。
完整受影响tunnel/严格Clippy/build/实际CLI最终结果由外部summary登记，未跑不写PASS。

受影响完整tunnel404通过、0失败、2原ignored。2个ignored保留browser客户端和
显式L04测量的原条件；原错误/source/native joins/严格EOF及其它原断言不变。

严格CLI Clippy/build、fmt/diff、readonly-policy实际manager1项均通过；localhost trap
仍断言Source发现零连接。host runner为5 PASS、0 FAIL、22 NOT_RUN，
real_device_executed=false。完整source/binary/log/commit关联另存summary，未变SDK/
core其它profile、feature/MSRV/平台/真实设备不由本项重新认证。

此项只交付这些有准确capacity的collectors及原错误引用/charge最后释放，不是原
error payload全量字节准入。Source/native/JoinError的任意payload创建端、anyhow/
Arc/watch内部、history/static registry/其它集合backing、manual desired/route与完整
freshness/刷新/复用/重启/manual epoch仍缺。试验辅助入口接收预先由fixture创建的
Vec，拒绝时以安全context退回原参数roots；不是生产Source准入流程或完整主项证据。
保留原native joins/严格EOF，不用Drop/单向flush/RST认证设备ACK。

账本仍23开发交付、6缺开发、L02待完整收口，有效30，Q04及全部子项排除。
Rust/Cargo串行，只本地提交，无子agent、push/发布/部署。

外部证据目录：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\12\01a13061-96bb-7482-9e21-b15d7a2e84aa\repair\l02-write-batches`。
