# L02 manual desired route共享owner · 2026-10-13

承接 `9983f9df5efd4cbc3d266e0cb512826de3d20c88`，实际工作树干净、分支保留，
541个构建来源文件/CLI hash与交接一致；补齐前项commit后的源码/二进制核对。

原Manual desired的RouteIntent改由一个已准入RetainedRoute持有原String buffer。
按String.capacity和inline owner size取得同一128MiB bank额度后才构建共享Arc。
Snapshot/回滚副本仅clone原Arc，不再复制route String或重复计费；Gate持同一bank。
Stop/Automatic状态保持inline，无新增route allocation或额度请求。

checked replacement先取得原state/pending destination锁并检验实际clock/grant，
再准入新route owner；未取得额度时旧状态不变。新旧route在替换期间同时占额度，
候选跨期回滚恢复原版本/Arc/pending，释放新owner。成功replace、stop或clear只
释放cell的一份所有权；caller旧Snapshot仍持有原route时不退款，最后owner才退款。

新增4项门禁实际通过：8KiB reserve/原buffer地址与多个Snapshot同一Arc/最后释放；
额度拒绝保留原version/watch/pending/Arc；后验取消回滚只退新owner；clear保留原epoch
Snapshot且zero-bank stop/reset可执行。既有fixture仅按内部route owner类型访问其
原内容，旧HTTP/native/错误/epoch/权限/EOF/joins断言不放宽。

受影响tunnel408通过、0失败、2原ignored；其余严格Clippy/build/实际readonly CLI/
host runner的实际结果见外部summary，未执行条件不改PASS。仍缺manual desired
listing的独立key/backing、manual请求/生产route其它副本、其它map/set/history/native
backing及Arc/watch/anyhow内部、原错误payload完整创建端准入与完整刷新规则。
本项不将route共享误称成整个manager heap或L02已完成，也不推导设备ACK。

账本23交付/6缺开发/L02待完整收口，有效30，Q04全部排除。无子agent、并行Cargo、
真实设备发现/操作、push/发布/部署。默认unit Source仍NoDevices，专用localhost
native/source fixtures原对象与收尾保持。

外部目录：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\12\01a13061-96bb-7482-9e21-b15d7a2e84aa\repair\l02-manual-route`。
