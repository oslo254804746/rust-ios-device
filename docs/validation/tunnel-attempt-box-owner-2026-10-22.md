# L02 native Attempt Box ownership · 2026-10-22

原Ownership::reserve用Box::new分配实际Attempt，外围scan/ensure F与native
固定entry费用不支付该独立Box。本项从原Registry byte bank精确准入实际
Attempt布局；首个空destination在Ownership构造/Ticket.commit内、Source
创建前准备，后续每个RemotePairing策略在各自原Ticket fence内先准入
具体destination，再reserve/start。旧stamp、slot、incarnation、原handle
与attach/complete/handoff方法保持，不把Box退款当作native join已完成。

Core增加兼容默认TunnelOwner::reserve_owned入口与栈上的
TunnelReservationOwner：保持同一个原Box，字段先Box后整笔permit，无
第二层Box/新任务/新bank/新限额。try_from_grant核对动态实际size与原
byte_bank指针；拒绝时原Box和grant一起返回。原公开reserve签名及默认
callback保留；原实现仍只调用原reserve。manager覆盖新入口，实际Core
InitialOwner/activation沿同一个owner交接。legacy reserve得到同一个
已准入Box，其permit并入同一个已计费Shared费用根，scope和每个原Box均持该根。
正常Box先释放后scope结束才退费；Box被mem::forget时该根仍保留整笔
费用，scope结束也不会提前退款。费用根自己的实际控制块同样从原bank
精确准入，未新增无费用wrapper。

CLI使用既有exact_backing创建空destination，同一个capacity1 Vec只在
放入实际Attempt后按相同layout转为原Box。新增一处局部unsafe说明
Vec/Box精确同布局、相同指针/对齐/容量且中间无失败步；Core owner不新增
unsafe。原Attempt增加一个可选legacy费用根字段，动态实际size照常准入；其
TunnelReservation方法实现逐字保持；原activation
正文只换参数所有者类型，生命周期/intent/server/Source生产分支及
Cargo.lock hash不变（source.rs仅测试默认实现迁移）；11个旧fixture文件仅迁移reserve→reserve_owned，
包括原failed-target→new-target、原scan history/typed source、原native
清理等旧断言保留；未替换为提前generic ResourceLimit通过。

新增9项测试（Core2、CLI7）：

- 对齐128的原Box保持原地址和动态布局；closed0正常及原Drop panic两条
  路径均在Box实际析构/释放后仍持有整笔费用，再全额退款。原panic payload保持。
- foreign-bank与N−1 grant拒绝后仍返回同一个原Box/permit；不提前Drop、
  不换bank，调用方按原顺序销毁后全额退款。
- 首个N−1/closed0在原native reserve/backing前拒绝，零受控allocation，
  native occupied0；首个已预付destination闭池0仍交接同一实际指针，
  首次reserve不再分配，Box Drop准确归还body及空native incarnation。
- 两份不同地址的Box按实际布局逐份扣费/退款；额外N−1尝试在新slot/start/backing前拒绝，
  第一个原Box指针/原inflight保留；legacy费用根strong count按scope+两份实际Box的3→2→1递减，
  两份Box费用及根控制块费用持续到原scope Drop，最终全额退款。
- 原Ticket下首个N−1产生before_source typed ResourceLimit且不占slot；
  funded后终态取消保留原预付destination/Reserved slot，closed0取消先于
  新分配，scope销毁才释放实际backing及incarnation。
- 两个实际本地userspace native runtime挂在不同原Attempt。闭池0分别Drop
  精确退各Box费用，第一原handle停止、第二仍活；complete/take_completed
  后第二保持原handle，实际lease退休后两份runtime的原始joins全部完成，原peer严格
  Ok(0) EOF，native closed+empty、任务基线与原bank全额恢复。

另以实际core_error边界验证原typed root及深层PermissionDenied IO地址
保持，private ResourceLimit仍可分类；使用持有原Anyhow的ReservationFailure
source，而非会隐藏具体root类型的into_boxed_dyn_error。

R1 strict检查仅命中新测试析构观察字段dead_code，改为下划线字段；R2/R3
修复新测试的相对模块路径及已有方法名，无allow/忽略/断言放宽。批量fixture
迁移时发现原é文本误转码，R4前已恢复。R4真实测试抓到额外
准入错误经旧core_error变为generic IO，修复typed root/source保留；同时
补齐legacy费用根的Box保留，防止忘记Box后scope结束提前退款。独立逐字
guard核对旧fixture仅回调入口变化；R5完整native抓到5个精确额度
fixture提前拒绝，补齐测试默认Source的owned回调，并在2处原临时峰值
公式中仅加真实Attempt布局、Source Drop后重持相同费用，保留原零余额/
原IO/history/handle/joins断言。R6全CLI1815/1失败/13原忽略，
唯一失败是observed-route N−1原前缀漏新Attempt；只在该公式加其实际size，
保留两份lease根分配与原observed_route::Admission断言，并新增先行门禁。round7全部24命令通过，全部Cargo/build主控串行：

- reservation-owner过滤：test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 1852 filtered out; finished in 0.01s
- initial-owner过滤：test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1853 filtered out; finished in 0.00s
- activation-owner过滤：test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 1849 filtered out; finished in 0.09s
- attempt-storage过滤：test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 1821 filtered out; finished in 0.01s
- native-attempt过滤：test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1828 filtered out; finished in 0.01s
- native-retirement过滤：test result: ok. 109 passed; 0 failed; 1 ignored; 0 measured; 1719 filtered out; finished in 43.33s
- scoped过滤：test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1828 filtered out; finished in 0.01s
- observed-route-prefix过滤：test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1828 filtered out; finished in 0.00s
- http-control过滤：test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 1824 filtered out; finished in 0.04s
- intent过滤：test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 1818 filtered out; finished in 1.45s
- future-storage过滤：test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 1825 filtered out; finished in 0.00s
- core-tunnel过滤：test result: ok. 159 passed; 0 failed; 0 ignored; 0 measured; 1695 filtered out; finished in 1.94s
- listener-owner过滤：test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 1851 filtered out; finished in 0.01s
- listener-owner-no-mdns过滤：test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 775 filtered out; finished in 0.04s
- reservation-owner-no-mdns过滤：test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 776 filtered out; finished in 0.01s
- cli-full过滤：test result: ok. 1816 passed; 0 failed; 13 ignored; 0 measured; 0 filtered out; finished in 244.59s

另含Core与CLI all-features/all-targets strict Clippy -D warnings、Core
no-default check（该配置有9项dead-code warning，非strict gate）、build/fmt/diff、真实只读manager1（Source/usbmux trap0）、
host5 PASS/0 FAIL/22 NOT_RUN，real_device_executed=false。703冻结源码无
漂移，23处源码差异（含3新增文件），Core本轮实际重验，Cargo.lock不变。
实际host及归档verified-ios.exe SHA-256：
b1133f95ee98c8b0141c844ed3039e00953f30cb8a3b6848e8ca5d2708c0f61d
完整命令/log/result、source delta、current/提交后rawGit proof与实际二进制：
C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-attempt-box-owner\round7

本项不支付原marker Arc/control block、opaque错误/heap、HTTP/framework/
task、SDK/config/credential/public Core Arc/Weak/watch、transport/allocator/
waker/waiter等其它原范围。未证明完整marker arena容量/同bank来源/控制峰值，
各模式endpoint恢复和native exposure事务也未关闭。128MiB仍是已知对象
准入池，非整个heap/RSS；未做Miri/其它平台/真实设备验证。有效30/Q04全排除、
23已交付/6缺开发/L02待全审计不变，仅本地提交并继续原七主项。
