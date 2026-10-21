# L02 publication Node final physical owner · 2026-10-21

原publication Node的两笔whole permit位于Box内部，覆盖root和canonical key；
字段排序能保持value/key结束后退款，但Box自身仍在permit字段之后才释放。
本项把两笔原费用移到OwnedNode的Box外，data字段先完成原Node析构与dealloc，
两费随后退款。没有第二个guard分配、预算池、T:Unpin约束或新生产unsafe。

Node的next与真实fixed slots、prepared、Change/Retired/Whole/Removed/Iter
统一使用同一个OwnedNode。只读Deref及模块私有node_mut保持原Node/key/value
地址和费用归属；没有向其它模块导出可swap整Node的DerefMut。两池完整费用
仍在任何key/root分配前取得，实际Node尺寸与key.capacity计量；4096固定槽
backing按实际Option<OwnedNode>宽度准入，不能沿用Box指针旧宽度。原table
entry ceiling、epochs、Source前目的地、原node/key复用、late rollback、
Selection恢复和Removed正常迭代析构保持，owned_publication_bytes读取外层
原费用。没有通过提前释放旧owner来容纳替换候选。

保留原7项publication测试并增强差一字节拒绝为零backing；新增3项验证：
两池整份借记早于实际key与Box且恰好原两处分配；align128/!Unpin原值在原址
析构，unused/removed及原析构panic三路径均在整个Box结束后、两笔信用退款
前触发cfgtest零尺寸sentinel；4096个真实intrusive节点在闭池零余量下恢复
原pointer/key/slot，再正常迭代释放，每个after-free观察仅看到此前已结束根
的信用，最后backing与held释放准确退满。原native metadata/freshness/IO/
actual joins及严格EOF不变；测试observer不改变其它metadata_vec observer。

14个主控串行门禁全部通过：publication10、retain3、publication集成10、
native metadata7、native freshness17、manual admission12、observed边界1、
strict CLI all-features/all-targets Clippy -D warnings、完整CLI1763 PASS /
0 FAIL / 13原ignored、build/fmt/diff、实际只读manager1（Source/usbmux trap0）、
host5 PASS / 0 FAIL / 22 NOT_RUN，real_device_executed=false。Core/Cargo未改，
复用55f21fe的Core证据；未执行Miri、其它平台或设备验证。子Agent仅准备外部
proposal，主控逐行复核并核对原/proposed SHA后应用、格式化与冻结；全部Cargo
及cargo build由主控执行。原会话观察handle后续不可用，已读取全部14份真实
result且核对command/exit0、无在途进程；没有因观察handle缺失重跑已完成门禁。
684冻结源码无漂移，2处源码差异，归档CLI与实际host报告SHA-256一致：
`9ae534c0acdf571c707baa6c5c9e400b2a5611b55f81ecd7b4b2e65d77fe3965`。
证据与提交后rawGit proof：`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-publication-body-final-owner\round1`。

opaque Anyhow/context外壳、freshness report、其它marker/Core private headers、
SDK/发现、HTTP/WS/配置/凭据、框架task/allocator及完整endpoint恢复仍按原范围
继续。128MiB为已准入已知对象池，非完整heap/RSS。L02未关闭；23交付/6缺开发/
L02待全审计、有效30及Q04全部排除不变。仅本地提交，无push或真实设备操作。
