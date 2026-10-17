# L02 original publication connection lifetime · 2026-10-17

同device/global epoch内替换row或完整扫描prune，原ticket仍有效，旧连接不能
依靠它结束。先以真实localhost TCP peer不主动关闭的fixture复现：替换与prune
两项结果均false，publication身份已变/row已移除，原ticket仍current。

publication身份现在是Arc<publication::Life>。payload含closed atomic、inline
Notify和独立两份permit，按实际size_of在Source前同时准入原global/published
bank，再Arc::new。锁定tokio1.50.0 Notify::new构造inline atomic和intrusive
waiters mutex，无新增waiter容器；Arc header/allocator/TaskCell仍是私有边界。
新quota拒绝保持原Resource cause，marker阻止默认discovery fallback。

ManagedTunnel Drop关闭且只关闭其原publication，再退役原native owner。
原移除/替换/rollback owner仍在table guard外析构；rollback保留旧row与原
publication，不把临时未提交的新身份当旧身份关闭。LocalTarget同guard捕获
record、原ticket和原publication。dial与upgrade同时受两种原寿命约束，dial
成功后再检查原publication。manual live保留同一row/运行时，因此不误关；
真正替换或移除会唤醒旧连接。closed flag每poll先查，覆盖close线程在atomic
store与notify之间被抢占的窗口。原ready错误跨close仍保持typed cause/指针。

scan-start snapshot继续共享原身份及其费用，关闭不等于销毁。最后snapshot/
connection释放才归还payload两池费用；更新精确native/freshness公式和Weak
析构证据，保留原worker joins、cache owner及strict Ok(0) EOF条件。snapshot
key原先clone后收费，本次改为实际u8 Vec先准入再复制，并校验原global bank；
allocation hook核对原目的地指针和收费时序，原source key/capacity保持。

新增9项覆盖实际body创建时两池已收费、任一池差一字节不创建/完整退款、最后
已关闭owner、预关闭零poll/挂起future Drop、atomic store早于notify时零额外
poll、ready原IO跨close指针、异池snapshot拒绝、真实替换/完整prune旧连接
strict EOF、manager构造拒绝Source/USB/paired0及不fallback、late替换rollback
恢复同原Arc且原连接继续传输。既有manual live/B隔离、原source错误/native
joins/EOF与freshness全组保持。这里只证明publication寿命，不认证物理freshness。

最终完整tunnel624PASS/0FAIL/2原ignored，upstream access10PASS；all-feature/
all-target CLI Clippy -D warnings、build/fmt/diff、实际只读manager1通过；
Source/usbmux trap0，host5PASS/0FAIL/22NOT_RUN，real_device_executed=false。
632个冻结源码无漂移。core/Cargo未改，沿用此前fb89530 core诊断7/8及strict
证据，不冒充重跑。CLI SHA-256：
`39c522d316d2ef193557afce50ba1247165e14f8363bd8439f8f5d9af9e28f98`。

最终日志/冻结清单/受测binary/current及提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-publication-lifetime\round4`。
red保留原[false,false]复现；round1旧fixture Arc<()>类型编译失败，改为Source
前建实际paid身份后round2初始5项通过；round3全部9项通过、full616/8FAIL为
新payload费用/原快照owner公式未更新。显式size_of公式、原快照最后释放和Weak
析构断言修正后，round4 native6及完整全部通过，未削弱原金额/joins/EOF。

remote proxy全局寿命、实际框架upgrade joins/TaskCell、received/relay/private
buffer/IO/error/Arc/allocator、初始config/client/credentials、URI、Source/
candidate创建、initial公开RSD目录与全部原刷新/复用/重启/manual epoch/选择/
prune全审计继续。128MiB非完整manager heap/RSS；L02不关闭，23交付/6缺开发/
L02待全审计、有效30和Q04全部排除保持。仅本地提交，无真实设备或发布。
