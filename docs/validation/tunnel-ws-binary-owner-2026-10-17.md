# L02 outgoing WebSocket Binary owner · 2026-10-17

TCP→WebSocket原先to_vec后交给send；局部permit不能覆盖取消后SplitSink.slot
或错误中仍存活的原Vec。本次实际u8 Layout在复制前准入，费用先进入endpoint
pending owner，再把原Vec交给sink。成功flush才归还这一原payload费用；
private serialized write buffer不由它覆盖。pending未完成时不能覆盖原费用或
再建第二个destination，128次单笔配额复用回归保持精确退款。

Endpoints按sink、receiver、pending排列；两个half及底层socket析构后才退款。
两个方向的future只借用它，取消不会先归还仍在slot/共享底层中的原payload费。
失败时先把原Result与费用放入Completion，再销毁endpoints。Completion原
Result在前、grant在后，错误携带原Frame/Vec时仍持费到错误销毁。只提供
借用Result，原anyhow typed downcast、source与指针保持，没有可脱离grant的
owned error导出或改变downcast的包装context。CLI私有bridge及对应fixtures
接线，公共SDK/API不变；production使用原manager全局bank。

锁定futures-util0.3.32的SplitSink先drop lock再drop slot，不能仅给底层socket
挂费；anyhow1.0.102 ContextError先drop context再drop error，也不能普通
context挂费。锁定tungstenite0.24把Binary Vec move进Frame，WriteBufferFull
可带回这个原Frame；成功format/flush后原Vec已释放。axum0.7.9转发该原错误，
未删除或重分类。原二进制规则、关闭期限、TCP EOF及背压处理保持。

新增7项覆盖实际创建前Layout/pointer、差一字节无分配、取消后inline slot与
共享底层两个half的析构次序、另一方向完成时pending原data退款、start/flush
错误原Vec/typed error/IO source指针和最后owner、成功flush重复复用配额。
真实localhost bridge在0-byte池拒绝新Binary复制并使原TCP peer strict Ok(0)
EOF；quota拒绝前已读TCP数据，连接关闭且不复用，不声称没有原TCP IO。
真实1-byte池+1-byte写buffer触发原WriteBufferFull，返回Frame仍是allocation
hook所见原Vec指针，socket/peer销毁后错误继续持费，最后Completion退款。

专项7PASS；完整tunnel610PASS/0FAIL/2原ignored，upstream access10PASS；
all-feature/all-target CLI Clippy -D warnings、build/fmt/diff、实际只读manager1
通过；Source/usbmux trap0，host5PASS/0FAIL/22NOT_RUN，
real_device_executed=false。629个冻结源码无漂移，core/Cargo未改，沿用此前
fb89530 core诊断7/8及strict证据，不冒充重跑。CLI SHA-256：
`97f3e13f5b09915654709b5ee967aedc84d4c2c2a8729d6c50bc00ab17d25879`。

全部日志、冻结清单、受测binary/current和提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-ws-binary-owner\round1`。
本轮同一冻结源码通过全部检查，未复用本轮不同源码的PASS。

本项仅覆盖新建outgoing Binary Vec，received framework帧、WS relay、私有
写buffer/IO/error/Arc/allocator/TaskCell、upgrade取消/joins、初始config/client/
credentials、URI、Source/candidate创建、initial公开RSD目录及原完整physical
freshness/刷新/复用/重启/manual epoch/选择/prune仍继续。128MiB不是完整
manager heap/RSS上限；L02不关闭，23交付/6缺开发/L02待完整审计、有效30
及Q04全部排除保持。仅本地提交，无真实设备、push、发布或部署。
