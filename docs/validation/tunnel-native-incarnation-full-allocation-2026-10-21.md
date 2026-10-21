# L02 native incarnation full allocation · 2026-10-21

原 Registry Key/Entry 的 private incarnation 使用 Arc<()>，其实际控制块没有
进入已知对象池。本项接入现有 shared_allocation::Shared<()>，按应用声明的
完整实际 layout（strong counter、原费用及padding）准入；不估算标准 Arc
私有头、不另建池或 guard。sealed、slot capacity、checked ordinal 的原拒绝
顺序保持；随后先取得原 byte_bank 整份 grant 并创建实际根，最后才修改
issued/slots。任何字节或 backing 拒绝都不消耗 ordinal、slot 或触达 Source。

仅 Key/Entry incarnation 创建、clone、entry lookup 和两处 refresh fence 的
指针比较接线改变。OwnershipStamp 的 global/device、原 TunnelHandle、绝对
期限、退避和实际 joins 未改。动态费用不计入空 Registry structural_bytes，
也不计入 publication 子池；它独立跟随最后一个 Entry/Key owner。slot 清空
以后，旧 NativeLease、RefreshProof.key 或外部 Key 仍可保持原 allocation，
原 bank 只能在整个实际根 dealloc 后退款。旧 key 不会影响复用 slot 的新根。

新增3项必要测试：0/差一字节拒绝在任何 backing 前，issued/retired/slots及
原 slots 地址不变；exact grant 仅产生一个实际布局、先完整借记、原 bank及
原 Entry/Key 地址保留，空 reservation 释放后旧 Key 继续持费；闭池零可用下
8个原 Key 并发最后释放只退款一次；实际 manager publication destination 后
差一字节拒绝使 Source/USB/paired/default fallback、retry/失败写入均为零，
原生 slot 为空，原已付费 publication backing 与最终清理退款准确恢复。

增强原 stale lease/slot reuse：实际 joins 清空旧 slot 后旧根和费用仍保留；
新根同 slot 且指针不同，drop 旧根只退旧费用，新 runtime 继续 active/alive。
ordinal overflow 在闭池下仍优先返回原原因，禁止任何根 backing与Source poll。
保留 report fixture 手动持有的旧 Key：history/root/lease/Registry 结束后仍扣
该根费用，最后显式 drop(key) 才退满，没有提前抹掉旧 owner。其它预算只在
实际根存在时计入 global：joins 已完成而 table 还持 Key 时仍保持原零余量；
最后 Key 结束的路径才增加退款。publication 和固定 structural 费用保持。

主控复核修正了新 manager observer 的边界：先记录 probe.prepare 的完整真实
publication layout 前缀，实际仅允许该前缀且必须全部出现，后面任何额外
backing 都拒绝。它不能在 publication_before_prepare callback 刚触发时就
误拦必需的已付费 publication 分配。原 observed host-minus-one 夹具同样只
加实际 incarnation 前缀，保留原 Admission、两个 lease backing 及零Source
断言。原 IO/Work 指针、容量、严格 Ok(0) EOF 和原 native joins 全部保留。

14个验收命令通过：incarnation筛选7、retirement40、ownership2、manual
admission12、observed边界1、native metadata7、Source constructor3（不轮询
真实NativeSource）、strict CLI all-features/all-targets Clippy -D warnings、
完整CLI1753 PASS / 0 FAIL / 13原ignored、build/fmt/diff、实际只读manager1
（Source/usbmux trap0）、host5 PASS / 0 FAIL / 22 NOT_RUN，
real_device_executed=false。Core/Cargo未改，既有e8b5129 Core证据复用，未
声称重跑Core测试。未执行Miri、其它平台或真实设备验证。

用户允许子Agent后，本项由子Agent修改并静态自审、主控复核与格式化；全部
Cargo命令及cargo build仅由主控串行执行。684冻结源码无漂移，10处源码差异；
归档受测CLI与实际host报告SHA-256一致：`04d2c9b85970df01cd7527bb9e983dbe61f5a3e67d13142374a259e982818b72`。
日志、source delta、verified-ios.exe、current及提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-native-incarnation-full-allocation\round1`。

global/device/intent/retry等其它marker、opaque Error/Anyhow/context、Core
PreparedRsdOwner私有头、SDK/发现输出、HTTP/WS/配置/凭据、框架IO/task/
allocator与完整freshness仍按原范围继续。owned current真实服务socket与实际
restart正向证据尚待后续应用和运行，外部proposal不是PASS。128MiB为已准入
已知对象池，非完整heap/RSS。L02未关闭；主项23交付/6缺开发/L02待全审计、
有效30与Q04全部排除不变。仅本地提交，无真实设备、push或发布。
