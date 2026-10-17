# L02 original Source future allocation · 2026-10-17

外层candidate F已持费不覆盖NativeSource::connect_owned另建的BoxFuture。
现在共用future_storage::PaidFuture的精确F Layout分配，Dynamic把同一个
Pin<Box<F>>直接unsize为dyn Future，不再Box一个包装future。原Box在前、
独立OwnedSemaphorePermit在后；未来取消/完成时原F析构、allocation释放后
才归还费用。没有分配后再size_of_val补费，没有unsafe pin projection。
旧federation/scan typed路径继续共用同一原分配实现。

私有Source::connect_owned增加原bank参数，返回Result<Dynamic<Result<Device>>>：
外层Err是未运行Source的构造拒绝，内层仍为原async Source结果。NativeSource、
默认owned wrapper和全部覆盖此方法的fixture均在Box前创建实际准入的具体F，
未用raw fixture适配冒充production已收费。fixture内部旧connect/SDK/private
future仍是独立边界；本项不是全部Source操作/future已经持费的声明。

manager先完成observed route与原Source Box准入，再读retry版本/启动attempt；
构造拒绝使用原ownership::before_source context，不poll、不emit start、不走
default paired fallback。首次原Source poll才沿原逻辑报告开始。Ready后显式
drop原Source owner，再做设备/输出准入；原Err类型/指针不变。原返回Device/
Err自有分配不由已经结束的Source frame费用覆盖，不把短sidecar当最后owner。
公共SDK/API、原epoch/absolute deadline/native lease/原task joins不变。

新增6项覆盖：实际allocation hook/高对齐/!Unpin原地址及取消析构先于退款；
差一字节零分配/零poll；原typed IO pointer和独立paid输出离开Source Box；
ZST零分配；生产NativeSource四种route的实际constructor原指针，funded、零
配额与差一字节拒绝，始终不poll（Trap owner禁止reserve/真实设备）；实际
manager构造拒绝connect/USB/paired0、receipt0及部分destination精确退款。

原native metadata fixture新增实际临时峰值：用同一LocalSource构造器分配
但不poll，测量真实费用并完整退款后加入预算。测试专用零大小AfterDrop字段
在原Box/permit字段后运行，仅在已释放时占用测得的额外额度，模拟其它pool
使用以恢复原zero-free-byte metadata边界；不是生产延迟退款或虚构native
成本。原global/published精确金额、原worker joins、strict Ok(0) EOF保持。

最终完整tunnel648PASS/0FAIL/2原ignored，upstream access10PASS；all-feature/
all-target CLI Clippy -D warnings、build/fmt/diff、实际只读manager1通过；
Source/usbmux trap0，host5PASS/0FAIL/22NOT_RUN，real_device_executed=false。
640个冻结源码无漂移，core/Cargo未改，复用既有core证据不冒充重跑。
CLI SHA-256：`ee67a959d2ae25ebc34dab72cff87a5b2e5df057e147b5655c09c81aec3ea02a`。

最终日志、source delta、受测binary/current与提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-source-future\round2`。
round1专项6通过，full644/4FAIL是旧native fixture没给新Source Box临时峰值
留费用，提前在constructor拒绝。按实际构造器测量补预算并维持原零配额压力后，
round2 native6和完整全部通过，原失败日志/金额/EOF断言保留。

下一独立分配包括返回ConnectedDevice的Box、Source raw host/UDID、其它Source/
SDK future、初始RsdHandshake目录逃逸，以及HTTP/WS/URI/client/config/credential、
私有IO/error/Arc/allocator/TaskCell和实际框架joins、原freshness完整审计。
128MiB非完整heap/RSS；L02不关闭，23交付/6缺开发/L02待全审计、有效30及
Q04全部排除保持。仅本地提交与隔离fixtures，无真实设备、push或发布。
