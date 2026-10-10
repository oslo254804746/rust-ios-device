//! Adapter-only host checks. Installed-extension protocol evidence is produced
//! by scripts/python-device-query.py, which cannot substitute Python results.
use super::*;
use std::error::Error;

#[test]
fn binding_slots_are_independent_and_roll_back_on_refusal() {
    let pool = Admission::new(1).unwrap();
    let job = pool.try_job().unwrap();
    assert!(pool.try_job().is_none());
    let observer = pool.try_observer().unwrap();
    let close_observer = pool.try_observer().unwrap();
    assert!(pool.try_observer().is_none());
    assert_eq!(pool.native.available(), 1);
    drop(close_observer);
    drop(observer);
    drop(job);
    assert_eq!(pool.available.load(Ordering::Acquire), 1);
    assert_eq!(pool.observers.load(Ordering::Acquire), 2);
    for limit in [0, 65, usize::MAX] {
        assert!(Admission::new(limit).is_err());
    }
}

#[test]
fn only_explicit_numeric_routes_and_finite_fields_are_accepted() {
    for field in ["ProductType", "ProductVersion", "DeviceName", "root"] {
        assert_eq!(selector(field).unwrap().1, field);
    }
    for field in ["", "HostID", "GetValue", "root\0", "DeviceName/Domain"] {
        assert!(selector(field).is_err());
    }
    assert!(endpoint_parse("tcp://127.0.0.1:1234").is_ok());
    assert!(endpoint_parse("tcp://[::1]:1234").is_ok());
    for endpoint in [
        "tcp://localhost:1234",
        "tcp://127.0.0.1:0",
        "",
        "file://credential",
    ] {
        assert!(endpoint_parse(endpoint).is_err());
    }
    for seconds in [f64::NAN, f64::INFINITY, -1.0] {
        assert!(duration(seconds).is_err());
    }
    assert!(deadline(0.0).is_ok());
}

#[test]
fn receipt_keeps_original_owned_error_and_zero_budget_can_read_joined_result() {
    let pool = Admission::new(1).unwrap();
    let job = Arc::new(Job {
        admission: pool,
        field: "DeviceName",
        cancel: OperationCancel::new(),
        result: Mutex::new(None),
        changed: Condvar::new(),
    });
    assert!(job.wait(Some(Instant::now())).is_err());
    job.publish(Err(Failure::Io(std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "owned-private-device-credential",
    ))));
    let first = job.wait(Some(Instant::now())).unwrap();
    let second = job.wait(Some(Instant::now())).unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(
        ios_core::classify_error(first.as_ref().as_ref().unwrap_err()),
        ErrorClass::PermissionDenied
    );
    pyo3::prepare_freethreaded_python();
    Python::with_gil(|py| {
        let error = observation(py, "DeviceName", first.as_ref()).unwrap_err();
        assert_eq!(
            error
                .value_bound(py)
                .getattr("ios_error_class")
                .unwrap()
                .extract::<String>()
                .unwrap(),
            "permission_denied"
        );
        assert!(!error.to_string().contains("owned-private"));
        assert!(error.cause(py).is_none());
    });
}

#[test]
fn unpolled_binding_reservation_releases_real_lease_before_publishing() {
    let pool = Admission::new(1).unwrap();
    let pair = PairRecord::from_bytes(br#"<?xml version="1.0"?><plist version="1.0"><dict><key>DeviceCertificate</key><data></data><key>HostCertificate</key><data></data><key>HostPrivateKey</key><data></data><key>RootCertificate</key><data></data><key>HostID</key><string>private-host</string><key>SystemBUID</key><string>private-buid</string></dict></plist>"#).unwrap();
    let owner = SessionOwner::new(pool.native.clone(), 1).unwrap();
    let device = ExistingTrustDevice::new(
        "private-udid".into(),
        Arc::new(pair),
        owner,
        endpoint_parse("tcp://127.0.0.1:1").unwrap(),
    )
    .unwrap();
    let session = DeviceSession {
        device,
        admission: Arc::clone(&pool),
        jobs: Mutex::new(Vec::new()),
    };
    let pending = session
        .prepare("DeviceName", 5.0, 8192, 256, OperationCancel::new())
        .unwrap();
    let job = Arc::clone(&pending.job);
    assert_eq!(session.device.active(), 1);
    assert_eq!(pool.native.available(), 0);
    assert_eq!(pool.available.load(Ordering::Acquire), 0);
    drop(pending); // Does not launch a task or touch the endpoint.
    assert!(job.done());
    assert_eq!(session.device.active(), 0);
    assert_eq!(pool.native.available(), 1);
    assert_eq!(pool.available.load(Ordering::Acquire), 1);
    assert!(job
        .result_lock()
        .as_ref()
        .unwrap()
        .as_ref()
        .as_ref()
        .unwrap_err()
        .source()
        .is_none());
    session.seal().wait_until(Instant::now()).unwrap();
    session.seal().wait_until(Instant::now()).unwrap();
}

#[test]
fn async_adapter_cancellation_timeout_error_priority_and_weak_lifetime() {
    pyo3::prepare_freethreaded_python();
    Python::with_gil(|py| {
        let adapter = helpers(py).unwrap();
        let check = PyModule::from_code_bound(
            py,
            r#"
import asyncio
import gc
import weakref

# These receipts test only Python observation, not native transport/result APIs.
class Receipt:
    def __init__(self, error=None):
        self.done = False
        self.error = error
        self.cancelled = False
        self.finished = 0
    def cancel(self): self.cancelled = True
    def _observer_finished(self): self.finished += 1
    def result(self, cleanup_timeout=0.0):
        if self.error is not None: raise self.error
        return 'owned adapter observation'
    def wait(self, cleanup_timeout=0.0):
        if self.error is not None: raise self.error

async def check(adapter):
    receipt = Receipt()
    future = adapter.start_query(receipt, 1.0)
    assert future.cancel('owned cancellation') and receipt.cancelled
    assert not future.done() and not future.cancelled()
    receipt.done = True
    try: await future
    except asyncio.CancelledError as error:
        assert error.ios_error_class == 'cancelled'
        assert error.ios_cleanup_complete and error.query_receipt is receipt
    else: raise AssertionError('native join cancellation must propagate')
    assert receipt.finished == 1

    receipt = Receipt()
    future = adapter.start_query(receipt, 0.0)
    future.cancel()
    try: await future
    except TimeoutError as error:
        assert not error.ios_cleanup_complete and error.query_receipt is receipt
    else: raise AssertionError('unfinished original join must time out')
    assert not receipt.done and receipt.cancelled and receipt.finished == 1

    receipt = Receipt()
    future = adapter.start_receipt(receipt, 0.0)
    try: await future
    except TimeoutError as error:
        assert error.ios_error_phase == 'device query receipt'
        assert not error.ios_cleanup_complete and error.query_receipt is receipt
    else: raise AssertionError('receipt cleanup wait must stay bounded')
    assert receipt.cancelled and receipt.finished == 1

    original = RuntimeError('safe native failure')
    original.ios_error_class = 'protocol'
    receipt = Receipt(original)
    future = adapter.start_query(receipt, 0.0)
    receipt.done = True
    future.cancel()
    try: await asyncio.ensure_future(future)
    except RuntimeError as error: assert error is original
    else: raise AssertionError('original native error was discarded')

    receipt = Receipt()
    future = adapter.start_query(receipt, 1.0)
    reference = weakref.ref(future)
    await asyncio.sleep(0)
    del future
    gc.collect()
    await asyncio.sleep(0.015)
    assert reference() is None and receipt.cancelled and receipt.finished == 1

    receipt = Receipt()
    original = ValueError('caller body error')
    future = adapter.start_close(receipt, 0.0, True, original)
    try: await future
    except TimeoutError as error: assert error.__context__ is original
    else: raise AssertionError('close original error was discarded')

    receipt = Receipt()
    loop = asyncio.get_running_loop()
    previous = loop.get_task_factory()
    def refuse(loop, coro): raise MemoryError('owned observer allocation refusal')
    loop.set_task_factory(refuse)
    try:
        try: adapter.start_query(receipt, 1.0)
        except MemoryError: pass
        else: raise AssertionError('observer refusal must fail')
    finally: loop.set_task_factory(previous)
    assert receipt.cancelled and receipt.finished == 1

def run(adapter): asyncio.run(check(adapter))
"#,
            "device_adapter_test.py",
            "device_adapter_test",
        )
        .unwrap();
        check.getattr("run").unwrap().call1((adapter,)).unwrap();
    });
}
