# L02 actual scan future window · 2026-10-17

原buffer_unordered(4)的实际future节点/执行payload没有由候选input费用覆盖。
现在owned_window使用实际KnownVec<Slot<F>>数组和精确F Layout buffer；至多
四个原future共用parent Context，不创建collector私有child-waker集合或任务。
Slot保留running Pin<Box<F>>或尚未初始化的AdmittedVec<F>(len0/capacity1)。
最终F放入原allocation后用共用PaidFuture::from_backing转换为同Layout Box，
没有额外payload分配，也没有unsafe pin projection。原F析构后才归还其费用。

初始已知USB窗口的真实slot/buffer在paired discovery前预付，独立保留跨越
不完整paired视图。后来有完整paired输入才按实际额外slot/buffer扩充。这样
paired准入失败不能夺走原USB执行存储；原publication、发现顺序和部分失败
继续规则保持。零候选不分配frame，空Vec<F>无F初始化、无child poll。
typed factory把预付buffer与真正async函数F绑定，不使用估算future类型。

所有初始body在首个child poll前准入；后来原child Ready后释放其Box/费，
可为下一个原输入申请实际body。补充窗口准入失败实际销毁pending和未poll的
queued futures，不只drop一个借用；原batch.finish保留错误收尾。child自己的
Err仍是原输出而非collector failfast，原typed pointer/IO类别保持。轮询允许
后来的ready项先完成，最多四并发；原connection permit、whole/child绝对期限、
native joins/strict EOF与批次写入仍走原接线，没有改成串行连接。

KnownVec的原IntoIter增加ExactSizeIterator，原data/backing grant仍保留至
iterator销毁；其原测试补len断言。实际请求前没有再建一套候选图。
取舍是最多四个独立F Box、较大的显式Slot和每wake最多四次child poll，均按
实际Layout收费；没有声称CPU/RSS改善。private Source/SDK Box、根任务/waker/
TaskCell、Arc header/allocator存储不由这些application payload费用覆盖。

新增8项覆盖真实slot/F费用在allocation hook时已持有、四并发与乱序完成、
!Unpin原地址、初始slot/最后F差一字节零poll/全部原输入Drop、补充拒绝销毁
pending/queued、caller Drop、原child错误/parent waker与后续输出、paid结果
独立于collector、empty零配额，以及预付实际指针在零剩余配额时成为同一
pinned F且析构先于退款。实际扫描按生产F大小差一字节拒绝，3个body已创建
但所有connect调用0、native空、ready不变；USB discovery1，paired尚未运行0。

最终完整tunnel642PASS/0FAIL/2原ignored、upstream access10PASS；all-feature/
all-target CLI Clippy -D warnings、build/fmt/diff、实际只读manager1通过；
Source/usbmux trap0，host5PASS/0FAIL/22NOT_RUN，real_device_executed=false。
637个冻结源码无漂移，core/Cargo未改，沿用既有core证据不冒充重跑。
CLI SHA-256：`cc0959054a1807513599289f35a8764c02455800bd233af3cc5437a576cd96ce`。

最终日志/冻结清单/受测binary/current与提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-scan-window\round5`。
round1初始6项通过；round2 full639/2FAIL揭示窗口申请过晚破坏USB部分连续性。
前移真实buffer预付后round3专项7通过；round4 full639/3FAIL为旧精确quota
fixture未留新frame费用。用原async函数返回类型实际分配测量、完整退款后把
金额加入公式，不硬编码layout；原paired计数/partial USB/退款断言不放宽。
round5完整通过；原失败日志保留，未把失败轮算PASS。

Source/candidate原文本创建、HTTP/WS/URI/client/config/credential、原框架task
joins/private IO/error/Arc/allocator/TaskCell、initial公开RSD目录与全部原
freshness/刷新/复用/重启/manual epoch/选择/prune全审计继续。128MiB非完整
heap/RSS，L02不关闭，23交付/6缺开发/L02待完整审计、有效30和Q04全部排除
保持。仅本地提交和隔离fixtures，无真实设备、push、发布或部署。
