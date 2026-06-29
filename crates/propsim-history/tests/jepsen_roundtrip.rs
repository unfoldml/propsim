//! Round-trip and golden-shape tests for the Jepsen history interchange.

use propsim_history::{Clock, Function, History, OpEntry, OpKind, ProcessId, Time, Value};

fn sample_history() -> History {
    History::new(vec![
        OpEntry {
            index: 0,
            time: Time::virtual_nanos(0),
            kind: OpKind::Invoke,
            process: ProcessId(0),
            f: Function::new("txn"),
            // [[:append :x 1] [:r :y nil]]
            value: Value::List(vec![
                Value::List(vec![
                    Value::keyword("append"),
                    Value::keyword("x"),
                    Value::Int(1),
                ]),
                Value::List(vec![Value::keyword("r"), Value::keyword("y"), Value::Nil]),
            ]),
        },
        OpEntry {
            index: 1,
            time: Time::virtual_nanos(1_500_000),
            kind: OpKind::Ok,
            process: ProcessId(0),
            f: Function::new("txn"),
            value: Value::List(vec![
                Value::List(vec![
                    Value::keyword("append"),
                    Value::keyword("x"),
                    Value::Int(1),
                ]),
                Value::List(vec![
                    Value::keyword("r"),
                    Value::keyword("y"),
                    Value::List(vec![Value::Int(7), Value::Int(8)]),
                ]),
            ]),
        },
        OpEntry {
            index: 2,
            time: Time::wall_nanos(42),
            kind: OpKind::Info,
            process: ProcessId(3),
            f: Function::new("write"),
            value: Value::Str("hi".into()),
        },
    ])
}

#[test]
fn edn_round_trips() {
    let h = sample_history();
    let edn = h.to_jepsen_edn();
    let back = History::from_jepsen(&edn).expect("parse EDN");
    assert_eq!(
        h, back,
        "EDN round trip must be lossless\n--- edn ---\n{edn}"
    );
}

#[test]
fn json_round_trips() {
    let h = sample_history();
    let json = h.to_jepsen_json();
    let back = History::from_jepsen(&json).expect("parse JSON");
    assert_eq!(
        h, back,
        "JSON round trip must be lossless\n--- json ---\n{json}"
    );
}

#[test]
fn clock_provenance_is_preserved() {
    let h = sample_history();
    let back = History::from_jepsen(&h.to_jepsen_edn()).unwrap();
    assert_eq!(back.entries()[0].time.clock, Clock::Virtual);
    assert_eq!(back.entries()[2].time.clock, Clock::Wall);
}

#[test]
fn golden_edn_shape() {
    // A minimal entry renders to the Jepsen operation-map shape.
    let h = History::new(vec![OpEntry {
        index: 0,
        time: Time::virtual_nanos(0),
        kind: OpKind::Invoke,
        process: ProcessId(0),
        f: Function::new("read"),
        value: Value::Nil,
    }]);
    let edn = h.to_jepsen_edn();
    assert_eq!(
        edn.trim(),
        "{:index 0, :time 0, :clock :virtual, :type :invoke, :process 0, :f :read, :value nil}"
    );
}

#[test]
fn ingests_foreign_history_without_clock_tag() {
    // A Jepsen store/ history has no :clock tag; it must default to wall-clock.
    let foreign = "{:index 0, :time 100, :type :ok, :process 1, :f :read, :value 5}";
    let h = History::from_jepsen(foreign).expect("parse foreign");
    assert_eq!(h.entries().len(), 1);
    assert_eq!(h.entries()[0].time.clock, Clock::Wall);
    assert_eq!(h.entries()[0].time.nanos, 100);
    assert_eq!(h.entries()[0].value, Value::Int(5));
}

#[test]
fn ingests_single_vector_of_maps() {
    let vec_form = "[{:index 0, :time 0, :type :invoke, :process 0, :f :read, :value nil} \
                     {:index 1, :time 5, :type :ok, :process 0, :f :read, :value 9}]";
    let h = History::from_jepsen(vec_form).expect("parse vector form");
    assert_eq!(h.entries().len(), 2);
    assert_eq!(h.entries()[1].value, Value::Int(9));
}
