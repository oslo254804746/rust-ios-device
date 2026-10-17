# L02 local upgrade original epochs · 2026-10-17

原local on_upgrade仅监听principal.revoked；原TCP peer不主动关闭时，stop/
clear/shutdown不能结束该连接。先以真实localhost HTTP/WS与保留TCP peer
复现：三个操作后的关闭结果均false，随后主动drop客户端才能使原TCP EOF。
该红灯日志保留，不由原native IO最终关闭推导manager已主动取消。

现在record与其原publication row的_ticket在同一表guard下捕获为LocalTarget，
不在复制旧record后获取可能属于重启的新ticket。Ticket::for_connection只移除
desired-version fence，保留原device/global epoch。手动live shortcut改变
desired仍可使用同一物理运行时，因此不误关原数据连接；stop(A)、clear和
terminal shutdown则分别使原epoch失效。

原TCP dial和整个local桥接都由此原ticket运行。dial后再同步commit检查，
过期成功stream不进入upgrade。原15s connect timeout覆盖dial/preamble，
原Elapsed状态/类别、原IO cause和原权限撤销规则保持。网络等待不持有表guard
或设备serial锁。普通get_record复用同一同步projection/prune流程，不额外
捕获ticket；record复制拒绝仍保留原CopyAdmission。

桥接返回的Completion放在Ok中穿过ticket runner，保留其原typed error/
source/pointer和payload grant。取消pending桥接实际销毁原future/endpoints；
若桥接已经在同一poll取得原payload错误后跨clear，该原错误继续返回，费用
跟随其最后owner，不被新取消替换。

新增5项覆盖：红灯同一真实stop/clear/shutdown路径转绿且原TCP strict Ok(0)
EOF、无额外发现/Source；旧record与新restart ticket不同incarnation且旧操作
不poll；真实双连接manual live仍可传输、停止A后B继续、shutdown关闭B与原
peer；实际Pending shared sink被三种epoch事件销毁/退款；ready payload
error同poll跨clear保留原类型/IO指针/最后费用。Pending测试使用受控sink，
未把真实idle网络fixture称为实际OS背压测量。测试peer的原任务handle实际
await，并非用通知代替join；生产axum框架upgrade task本项不宣称已纳管join。

专项5PASS；完整tunnel615PASS/0FAIL/2原ignored，upstream access10PASS；
all-feature/all-target CLI Clippy -D warnings、build/fmt/diff、实际只读manager1
通过；Source/usbmux trap0，host5PASS/0FAIL/22NOT_RUN，
real_device_executed=false。630个冻结源码无漂移，core/Cargo未改，沿用此前
fb89530 core诊断7/8和strict证据，不冒充重跑。CLI SHA-256：
`1a16abc133f369b1b364be8d839122fa1791d6cff8dd8e024b2f68bc3ee859eb`。

最终证据（日志/冻结清单/受测binary/current及提交后rawGit proof）：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-upgrade-epoch\round2`。
red目录保留原三个false复现；round1保留新代码对已有anyhow Result重复包装
造成的编译错误，去掉多余包装后round2全部通过，未修改红灯断言或关闭预算。

同incarnation replacement/prune的原publication寿命、remote proxy global
关闭、框架upgrade实际joins/TaskCell、received/relay/private buffers、初始
config/client/credentials、URI、Source/candidate创建、initial公开RSD目录及
全部原physical freshness/刷新/复用/重启/manual epoch/选择/prune审计继续。
本项不关闭整个upgrade或L02；128MiB非完整heap/RSS，23交付/6缺开发/L02待
全审计、有效30和Q04全部排除保持。仅本地提交，无真实设备或发布。
