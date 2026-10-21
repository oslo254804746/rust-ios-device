# L02 owned current catalogue / real service restart · 2026-10-21

原真实新service socket测试走legacy ConnectedDevice prepared slot，owned current
接线此前只有空目录ServiceNotFound路径证据。本项保留原6个prepared测试，追加
两个正向案例：owned current catalogue在实际服务停止并重启后，分别连接同端口
与变化后的端口，真实发送/接收不同代的fixture只读字节并严格结束IO。

固定IPv6 localhost RSD endpoint产生两次真实checked H2/XPC目录握手，沿公开
prepare_owned_existing_rsd_with_identity_at与原绝对deadline/byte bank创建目录。
使用explicit identity跳过主机身份查询；Core内部dial只连接明确的::1地址。
未调用CLI NativeSource、发现、usbmux、远程配对或设备路由。两个案例都先完成
第一代service IO，由client先shutdown，server与client分别断言read返回Ok(0)，
原peer task实际join、accepted socket与listener销毁之后再绑定第二代listener。
同端口案例真实重新bind旧地址；变端口案例另起新端口并在旧地址放accept trap。
没有sleep/retry或忽略断言掩盖rebind失败，没有伪造远端协议未提供的incarnation。

正向调用是OwnedConnectedDevice::connect_rsd_service。断言覆盖owned prepared
getter/swap/current选择、旧/新目录原body/rows/UID/features/modes缓冲区地址、
原TunnelHandle/地址/RSD端口，以及历史initial CoreError与深层原std::io::Error
地址保持。旧目录有独立export、displaced owner，第二代有独立current export；
关闭metadata bank后新service仍完成真实IO。按实际catalogue.known_bytes计量的
两个原grant持续至各自最后owner，逐步drop后准确退满。channel-only TunnelHandle
没有native worker，本测试不制造shutdown/joins证书；native实际joins另有CLI
现有夹具证据。Core PreparedRsdOwner的私有std Arc控制块与SDK服务输出分配
仍保留原未完成边界。

本项仅新增一个Core测试文件的两个测试。子Agent只在外部目录准备proposal，
未编辑repo或运行Cargo；主控复核全部拟议代码、核对原文件SHA与原字节前缀，
应用、格式化、冻结并串行执行全部Cargo与cargo build。

8个验收命令通过：owned筛选5（含新增2项与原owned接线）、prepared整组8、
strict Core all-features/all-targets Clippy -D warnings、CLI build、fmt/diff、
实际只读manager1（Source/usbmux trap0）、host5 PASS / 0 FAIL / 22 NOT_RUN，
real_device_executed=false。此次未重跑完整CLI，复用e9fc560的1753 PASS / 0 FAIL /
13原ignored证据；CLI及Core生产源码/Cargo未改。不声称执行完整Core测试或其它
平台/设备验证。684个冻结源码无漂移，唯一源码差异为上述Core测试文件。
当前CLI SHA-256与实际host报告及归档受测文件一致：`04d2c9b85970df01cd7527bb9e983dbe61f5a3e67d13142374a259e982818b72`。
日志、source delta、verified-ios.exe、current及提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-owned-current-service-restart\round1`。

RSD endpoint本身重新发现、manager Source成功后的publication peak拒绝/
wrong UID/缺失字段/失败后成功清理等完整freshness缺口继续；其它marker、
opaque Error/Anyhow/context、SDK/发现输出、框架IO/task/allocator等原范围继续。
128MiB为已准入已知对象池，非完整heap/RSS。L02不关闭；23交付/6缺开发/L02待
全审计、有效30及Q04全部排除不变。仅本地提交，无真实设备、push或发布。
