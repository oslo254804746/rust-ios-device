# L02 native RefreshProof actual root · 2026-10-23

原RefreshProof是inline值，G与foreign-origin收费采用size_of<Self>，
只能证明正逻辑keeper费，不能证明独立实际allocation。本项将原观察
字段放进实际共享根，费用保持到该根物理释放之后。

封闭RefreshProof wrapper持既有Strong<RefreshProofData, Origins2>。
Data保留原registry/key/stamp/handle/metadata/global_metadata/previous
字段次序；原G及两个origin permit在完整实际allocation中、T之外。
body_bytes读取完整Allocation类型含控制字段、所有permit、Data及
padding。没有逻辑sizeof(Self)费用、新unsafe、Clone/Weak/raw/mutable
出口或新NativeLease。Deref只借用Data，原proof方法体完整保持。

capture_refresh仍先持原Registry state lock并执行原slot/phase/精确
stamp/handle/metadata/previous及alive检查，再准入实际整笔G和原去重
authentic-origin费用，之后才clone原字段并实际分配。拒绝释放全部
暂存费用和捕获，不改变row/native状态。没有新Source、timer、authority
或retire动作。check/runtime/retain_failure/swap_metadata/payload_freshness
原锁、指针、历史、回滚及Drop协议保持，原观察结束不会停止runtime。
旧native proof精确测试只将逻辑sizeof公式替换为实际body_bytes；原
同bank/不同bank、slot重用、partial拒绝、real joins/EOF断言继续。

新增3项使用原retirement origin helpers的真实userspace duplex fixture：

- G/A1/A2逐个N−1与足额closed拒绝，根零backing，已暂存费用全部
  回退，原native active1/alive保持；随后原lease退休、actual owned
  joins、peer严格Ok(0) EOF和任务基线恢复，不进入CLI/SDK Source。
- 精确完整根费用全取到0后，确定性metadata_vec allocation拒绝
  观察原完整Layout并只调用一次；三个bank全额回退，原runtime仍
  alive/active，随后按原协议join。此为注入分配拒绝，不声称真实OOM。
- 成功只分配一次完整span，Data确位于该span，原runtime/metadata/
  global metadata/incarnation指针保持，原check通过。三个bank关闭
  后最后root物理free回调仍观察全部费用为0，之后准确退款；proof
  结束runtime仍alive且active1，原lease结束才退休并真实join/EOF。

主控whole-file sourceguard逐体重建原capture及全部proof方法，确认
只有实际根/type/费用公式与constructor glue迁移。原分配器unsafe、
NativeLease/Registry主体、stamp来源、Ticket/clock/native/history和
metadata所有权实现hash均保持。没有独立Agent最终实现批准，主控
完成源码复核和全部Cargo/build。冻结724源码后42门禁首轮通过：

- refresh-proof-root：test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 1888 filtered out; finished in 0.01s
- registry-all：test result: ok. 50 passed; 0 failed; 0 ignored; 0 measured; 1841 filtered out; finished in 0.18s
- origin-native：test result: ok. 7 passed; 0 failed; 0 ignored; 0 measured; 1884 filtered out; finished in 0.60s
- native-retirement：test result: ok. 110 passed; 0 failed; 1 ignored; 0 measured; 1780 filtered out; finished in 43.50s
- lifecycle-all：test result: ok. 336 passed; 0 failed; 0 ignored; 0 measured; 1555 filtered out; finished in 85.54s
- failure-global：test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 1886 filtered out; finished in 0.01s
- birth-bank：test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 1888 filtered out; finished in 0.00s
- connection-ticket：test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 1885 filtered out; finished in 0.05s
- publication-ticket：test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 1886 filtered out; finished in 0.01s
- reconnect：test result: ok. 53 passed; 0 failed; 0 ignored; 0 measured; 1838 filtered out; finished in 6.84s
- failure-table：test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 1877 filtered out; finished in 0.11s
- bookkeeping：test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 1882 filtered out; finished in 0.01s
- foreground-all：test result: ok. 16 passed; 0 failed; 0 ignored; 0 measured; 1875 filtered out; finished in 0.03s
- standalone：test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 1883 filtered out; finished in 0.04s
- cli-full：test result: ok. 1878 passed; 0 failed; 13 ignored; 0 measured; 0 filtered out; finished in 245.11s

其余upgrade/identity/shared weak/origin census/lifecycle/intent/manual/global/
error/failure slot/manager/HTTP/scan/ensure/future/publication-retain门禁也
通过；另有strict CLI all-features/all-targets Clippy -D warnings、
build/fmt/diff、实际只读manager1（Source/usbmux trap0），host5 PASS/
0 FAIL/22 NOT_RUN，real_device_executed=false。全部Cargo/build/fmt/test
主控串行，测试--locked/--test-threads=1。724冻结hash无漂移；3处仅
CLI差异（1生产/2测试、1新增测试文件）。Core/Cargo.lock与f41/ec25
保持；旧Core24证据沿用，本轮未重跑。实际host与verified-ios.exe：
SHA-256 f6e1086a3a6f6dff244a82df8542bdfb9c6832d7a26672640e99fc4c098d76a5
命令/logs/results、frozen/guard/delta/current/提交后raw HEAD proof：
C:\Users\wangbaofeng\.codex\visualizations\2026\10\16\01a143ff-7088-7b30-b229-ca062a4f255b\l02-refresh-proof-root\round1

本项证明actual RefreshProof根，不证明底层std Arc marker physical
arena或所有retainer/Weak/同步通知峰值。writer/snapshot/public/opaque/
Core/SDK/legacy/framework/task/waker/waiter/allocator/native/transport及
全部mode endpoint恢复/native exposure仍属原scope；128MiB是已知
对象准入池、非heap/RSS。无Miri/其它平台/真实设备结论。23交付/
6缺开发/L02待全审计、有效30和Q04全部排除保持，七主项继续。
