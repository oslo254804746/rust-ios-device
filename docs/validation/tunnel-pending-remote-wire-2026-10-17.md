# L02 pending remote wire cancellation · 2026-10-17

本次只新增隔离localhost回归，不修改生产行为。验证70cc006的global epoch
取消是否实际结束原pending IO，而不仅是application future返回。

原listener有界读取HTTP头：listing场景接受GET后不返回响应；handshake场景
先返回固定listing200/Connection:close，再接受WS请求但不发送101。分别
执行root clear/shutdown，且分别使用临时匿名client与持久配置认证client，
共8种组合。持久client保留到整个观察结束，两个阶段均核对Bearer字节。

8种组合均在1秒内取得root BAD_GATEWAY/原取消class，原尝试socket在1秒
观察期限内strict Ok(0) EOF；随后60ms观察窗没有新的accept，未扩大为无限期
零重试保证。原peer和caller实际task handle均await；失败cleanup分支才会
abort并await本fixture自己创建的handle，该分支不会计作EOF通过。原bank
全部恢复到请求前金额，整个流程不创建device gate。没有真实设备/Source IO。

结果是1项固定回归含8个场景PASS；all-feature/all-target CLI Clippy -D warnings、
fmt/diff通过，635个冻结源码无漂移。source-delta只含cfg(test)模块声明和
该新增fixture，故复用70cc006的生产build/实际只读manager Source/usbmux0、
host5PASS/0FAIL/22NOT_RUN以及此前完整tunnel629/0FAIL/2原ignored与上游10
证据，不把它们写成此次重跑。core/Cargo同样未变。受测生产binary保持：
`14b79233ff7bde8097a424c1e09686c85ee32c353405eaf59eed85621106a956`。

冻结清单、当前专项/Clippy、delta、binary、提交后rawGit proof：
`C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-pending-remote-wire\round2`。
round1匿名4种组合通过后扩展为round2完整8种，未改变原断言/预算。

观察到原wire关闭不等于已取得并join reqwest/hyper私有driver或axum upgrade
的实际handle；这些任务/TaskCell、其它平台/TLS pending IO、relay/received
buffer/private allocations及原完整freshness等仍继续。128MiB非完整heap/RSS，
L02不关闭，23交付/6缺开发/L02待全审计、有效30与Q04全部排除保持。
仅本地提交，无真实设备、push、发布或部署。
