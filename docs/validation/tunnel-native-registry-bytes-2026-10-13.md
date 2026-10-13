# L02 native registry本体与固定backing预算 · 2026-10-13

承接1bf6267并核对549构建来源/原CLI hash，本项在同一manager128MiB已知元数据
bank中，先取得实际sizeof Registry自有payload，再try_reserve_exact slots Vec，
按实际capacity×sizeof Option<Entry>取得backing份额，最后构建Arc与固定长度slots。
原4096联合owner上限、slot/incarnation、Reserved→Retired、原joins认证规则保持。
构造/额度失败向production manager、standalone tunnel、NativeScope及九处前台
调用方返回；没有在构造失败后进行Source discovery/启动native任务。

两份permit排在拥有的字段之后。移除/join一个entry不释放仍存在的Vec backing；
只有最后Registry owner，包括原Reservation，释放结构与backing才退款。Arc控制
块/allocator rounding未冒称已计量，runtime/native/原error私有payload仍是独立缺口。

测试bank换池只允许唯一Arc、未sealed且无owner/history。先原子取得两份新费用，
再替换旧permit，不新建/移动原slots、修改namespace或native identities。七处精确
quota fixture显式写入native.structural_bytes()+原目标余量；setter不自动增大bank。
原零余量/少一byte、原IO对象/EOF/joins/资源回收与epoch断言保持。新增4项门禁
检查实际容量、构造拒绝退款、Registry/原Reservation最后owner、换池原子性及
拒绝sealed registry改池。

新增4通过；整组tunnel428通过/0失败/2原ignored。严格CLI all-feature/all-target
Clippy、CLI build、串行fmt/diff通过。六组前台相关门禁viewer12/HID7/forward11/
WDA6/runtest11/runxctest10全部通过。readonly manager1通过且Source零连接；host
runner5 PASS/22 NOT_RUN，real_device_executed=false。实际前台fixture结果由外部
summary在运行结束后记录，不把pending/NOT_RUN改成PASS。

实际WDA/XCTest/crash三个localhost CLI fixture类31方法全部通过，包含正常及
失败/超时/取消/原stdout关闭、原PID/native/reader收尾和严格原peer EOF。源码冻结
后未修改Rust，受测CLI/hash与提交关联记录在summary和postcommit-proof。

本项没有对任意original dyn Error/JoinError panic/private/native payload建立完整
创建合同，也未覆盖observation/selection/drain scratch Vec、boxed ownership Attempt、
native report/context及其它retry/history/HashMap/Arc backing。完整freshness/复用/
重启/manual epoch仍须接线核查，原L02不关闭；账本23交付/6缺开发/L02待完整
收口，有效30，Q04及全部子项排除。所有执行为隔离localhost或unit，不进行真实
设备发现/操作，没有push/部署/发布。

外部目录：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\12\01a13061-96bb-7482-9e21-b15d7a2e84aa\repair\l02-native-registry`。
