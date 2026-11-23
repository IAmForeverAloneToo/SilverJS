//! The keyed collections and weak references (ECMA-262 sections 24.1 to
//! 24.4, 26.1 and 26.2): `Map`, `Set` with its set methods, `WeakMap`,
//! `WeakSet`, `WeakRef`, `FinalizationRegistry`, and the map and set
//! iterators. Each object holds its entries in a table ([`crate::table`])
//! named by its first internal slot.

use super::{CONFIGURABLE, Call, Entry, Name, What, Wk, entry, getter, method};
use crate::agent::Agent;
use crate::heap::{InternalCell, ObjectCell};
use crate::interp::{Abrupt, Res};
use crate::object::ObjectKind;
use crate::string::PropertyKey;
use crate::table::{Strength, Table};
use crate::value::{RefKind, Value};

/// `Map`.
pub(crate) const MAP: &[Entry] = &[
    method("groupBy", 2, group_by),
    entry(Name::Sym(Wk::Species), What::Getter(species), CONFIGURABLE),
];

/// `Map.prototype`.
pub(crate) const MAP_PROTO: &[Entry] = &[
    entry(Name::Str("constructor"), What::Constructor, super::HIDDEN),
    method("get", 1, map_get),
    method("set", 2, map_set),
    method("has", 1, map_has),
    method("delete", 1, map_delete),
    method("clear", 0, map_clear),
    method("entries", 0, map_entries),
    method("forEach", 1, map_for_each),
    method("keys", 0, map_keys),
    getter("size", map_size),
    method("values", 0, map_values),
    method("getOrInsert", 2, map_get_or_insert),
    method("getOrInsertComputed", 2, map_get_or_insert_computed),
    entry(Name::Sym(Wk::ToStringTag), What::Str("Map"), CONFIGURABLE),
    entry(
        Name::Sym(Wk::Iterator),
        What::Alias("entries"),
        super::HIDDEN,
    ),
];

/// `Set`.
pub(crate) const SET: &[Entry] = &[entry(
    Name::Sym(Wk::Species),
    What::Getter(species),
    CONFIGURABLE,
)];

/// `Set.prototype`.
pub(crate) const SET_PROTO: &[Entry] = &[
    entry(Name::Str("constructor"), What::Constructor, super::HIDDEN),
    method("has", 1, set_has),
    method("add", 1, set_add),
    method("delete", 1, set_delete),
    method("difference", 1, difference),
    method("clear", 0, set_clear),
    method("entries", 0, set_entries),
    method("forEach", 1, set_for_each),
    method("intersection", 1, intersection),
    method("isSubsetOf", 1, is_subset_of),
    method("isSupersetOf", 1, is_superset_of),
    method("isDisjointFrom", 1, is_disjoint_from),
    getter("size", set_size),
    method("symmetricDifference", 1, symmetric_difference),
    method("union", 1, union),
    method("values", 0, set_values),
    entry(Name::Str("keys"), What::Alias("values"), super::HIDDEN),
    entry(Name::Sym(Wk::ToStringTag), What::Str("Set"), CONFIGURABLE),
    entry(
        Name::Sym(Wk::Iterator),
        What::Alias("values"),
        super::HIDDEN,
    ),
];

/// `WeakMap.prototype`.
pub(crate) const WEAK_MAP_PROTO: &[Entry] = &[
    entry(Name::Str("constructor"), What::Constructor, super::HIDDEN),
    method("delete", 1, weak_map_delete),
    method("get", 1, weak_map_get),
    method("set", 2, weak_map_set),
    method("has", 1, weak_map_has),
    method("getOrInsert", 2, weak_map_get_or_insert),
    method("getOrInsertComputed", 2, weak_map_get_or_insert_computed),
    entry(
        Name::Sym(Wk::ToStringTag),
        What::Str("WeakMap"),
        CONFIGURABLE,
    ),
];

/// `WeakSet.prototype`.
pub(crate) const WEAK_SET_PROTO: &[Entry] = &[
    entry(Name::Str("constructor"), What::Constructor, super::HIDDEN),
    method("delete", 1, weak_set_delete),
    method("has", 1, weak_set_has),
    method("add", 1, weak_set_add),
    entry(
        Name::Sym(Wk::ToStringTag),
        What::Str("WeakSet"),
        CONFIGURABLE,
    ),
];

/// `WeakRef.prototype`.
pub(crate) const WEAK_REF_PROTO: &[Entry] = &[
    entry(Name::Str("constructor"), What::Constructor, super::HIDDEN),
    method("deref", 0, weak_ref_deref),
    entry(
        Name::Sym(Wk::ToStringTag),
        What::Str("WeakRef"),
        CONFIGURABLE,
    ),
];

/// `FinalizationRegistry.prototype`.
pub(crate) const REGISTRY_PROTO: &[Entry] = &[
    entry(Name::Str("constructor"), What::Constructor, super::HIDDEN),
    method("register", 2, register),
    method("unregister", 1, unregister),
    entry(
        Name::Sym(Wk::ToStringTag),
        What::Str("FinalizationRegistry"),
        CONFIGURABLE,
    ),
];

/// `%MapIteratorPrototype%`.
pub(crate) const MAP_ITERATOR_PROTO: &[Entry] = &[
    method("next", 0, map_iterator_next),
    entry(
        Name::Sym(Wk::ToStringTag),
        What::Str("Map Iterator"),
        CONFIGURABLE,
    ),
];

/// `%SetIteratorPrototype%`.
pub(crate) const SET_ITERATOR_PROTO: &[Entry] = &[
    method("next", 0, set_iterator_next),
    entry(
        Name::Sym(Wk::ToStringTag),
        What::Str("Set Iterator"),
        CONFIGURABLE,
    ),
];

/// What an iterator gives.
const KEYS: i32 = 0;
const VALUES: i32 = 1;
const ENTRIES: i32 = 2;

fn species(_: &mut Agent, c: Call) -> Res<Value> {
    Ok(c.this)
}

/// A key as a collection stores it: -0 as +0 (sections 24.1.3.9, 24.2.4.1).
fn canonical(key: Value) -> Value {
    if key.as_number() == Some(0.0) {
        Value::int(0)
    } else {
        key
    }
}

impl Agent {
    /// A new collection object of `kind` with prototype `proto` and an
    /// empty table of `strength`.
    pub(crate) fn new_collection(
        &mut self,
        proto: Value,
        kind: ObjectKind,
        strength: Strength,
    ) -> Res<Value> {
        let table = self.alloc_internal(InternalCell::Table(Box::new(Table::new(strength))))?;
        self.vm.temps.push(table);
        let o = self.alloc_object(ObjectCell::with_internal(proto, kind, vec![table]));
        self.vm.temps.pop();
        o
    }

    /// The table of `this`, a collection of `kind`, or the TypeError of
    /// `method` called on something else.
    fn table_of(&mut self, this: Value, kind: ObjectKind, method: &str) -> Res<Value> {
        let t = this
            .as_object()
            .and_then(|o| self.heap.objects.get(o))
            .filter(|c| c.kind() == kind)
            .and_then(|c| c.internal().first().copied());
        match t {
            Some(t) => Ok(t),
            None => {
                let text = self.display(this);
                Err(self.type_error(format!(
                    "Method {method} called on incompatible receiver {text}"
                )))
            }
        }
    }

    /// CanBeHeldWeakly (section 9.13): an object, or a symbol not in the
    /// global registry.
    pub(crate) fn can_be_held_weakly(&self, v: Value) -> bool {
        match v.as_reference() {
            Some((RefKind::Object, _)) => true,
            Some((RefKind::Symbol, _)) => !self.vm.registry.values().any(|&s| s == v),
            _ => false,
        }
    }

    /// A walker over table `t`: an iterator object that no script sees,
    /// moved as the table compacts.
    fn walker(&mut self, t: Value) -> Res<Value> {
        let w = self.alloc_object(ObjectCell::with_internal(
            Value::NULL,
            ObjectKind::SetIterator,
            vec![t, Value::int(0), Value::int(VALUES)],
        ))?;
        if let Some(o) = w.as_object() {
            self.heap.table_add_iterator(t, o);
        }
        Ok(w)
    }

    /// The next entry a map or set iterator (or walker) gives, or `None`
    /// once it is done.
    fn walk(&mut self, it: Value) -> Option<(Value, Value)> {
        let o = it.as_object()?;
        let (t, at) = {
            let s = self.heap.objects.get(o)?.internal();
            (*s.first()?, s.get(1)?.as_number()? as usize)
        };
        if t.is_undefined() {
            return None;
        }
        let found = self.heap.table_next(t, at);
        let cell = self.heap.objects.get_mut(o)?;
        match found {
            Some((i, k, v)) => {
                cell.set_internal(1, Value::number((i + 1) as f64));
                Some((k, v))
            }
            None => {
                cell.set_internal(0, Value::UNDEFINED);
                None
            }
        }
    }

    fn live_size(&self, t: Value) -> f64 {
        f64::from(self.heap.table(t).map_or(0, |t| t.live))
    }

    /// AddEntriesFromIterable (section 24.1.1.2) or the `Set` and
    /// `WeakSet` constructors' loop: `adder` of `target` called with each
    /// entry's key and value (`pairs`) or each value.
    fn add_from_iterable(
        &mut self,
        target: Value,
        iterable: Value,
        adder_name: &'static str,
        pairs: bool,
    ) -> Res<()> {
        let key = self.name_key(Name::Str(adder_name));
        let adder = self.get_value(target, key)?;
        if !self.is_callable(adder) {
            let (a, t) = (self.display(adder), self.display(target));
            return Err(self.type_error(format!(
                "'{a}' returned for property '{adder_name}' of object '{t}' is not a function"
            )));
        }
        self.vm.temps.push(adder);
        let r = (|| {
            let (it, next) = self.get_iterator_typed(iterable)?;
            self.vm.temps.extend([it, next]);
            let r = (|| {
                while let Some(item) = self.iterator_step(it, next)? {
                    let step = self.add_one(target, adder, item, pairs);
                    if let Err(e) = step {
                        if matches!(e, Abrupt::Throw) {
                            self.iterator_close_quiet(it)?;
                        }
                        return Err(e);
                    }
                }
                Ok(())
            })();
            let n = self.vm.temps.len();
            self.vm.temps.truncate(n.saturating_sub(2));
            r
        })();
        self.vm.temps.pop();
        r
    }

    fn add_one(&mut self, target: Value, adder: Value, item: Value, pairs: bool) -> Res<()> {
        if !pairs {
            return self.call_value(adder, target, &[item]).map(|_| ());
        }
        if item.as_object().is_none() {
            let text = self.display(item);
            return Err(self.type_error(format!("Iterator value {text} is not an entry object")));
        }
        let k = self.get_value(item, PropertyKey::Index(0))?;
        self.vm.temps.push(k);
        let v = self.get_value(item, PropertyKey::Index(1));
        let r = v.and_then(|v| self.call_value(adder, target, &[k, v]));
        self.vm.temps.pop();
        r.map(|_| ())
    }

    /// A collection's constructor: `kind` made from `new_target`'s
    /// prototype (or `default`'s), filled from the first argument.
    fn construct_collection(
        &mut self,
        c: Call,
        name: &str,
        kind: ObjectKind,
        default: fn(&crate::interp::realm::Realm) -> Value,
    ) -> Res<Value> {
        if c.new_target.is_undefined() {
            return Err(self.type_error(format!("Constructor {name} requires 'new'")));
        }
        let proto = self.prototype_from(c.new_target, default)?;
        let strength = match kind {
            ObjectKind::Map | ObjectKind::Set => Strength::Strong,
            _ => Strength::Weak,
        };
        let o = self.new_collection(proto, kind, strength)?;
        let iterable = self.arg(c, 0);
        if iterable.is_nullish() {
            return Ok(o);
        }
        self.vm.temps.push(o);
        let r = match kind {
            ObjectKind::Map | ObjectKind::WeakMap => {
                self.add_from_iterable(o, iterable, "set", true)
            }
            _ => self.add_from_iterable(o, iterable, "add", false),
        };
        self.vm.temps.pop();
        r.map(|()| o)
    }

    /// A map or set iterator over table `t`, giving `what`.
    fn collection_iterator(&mut self, t: Value, map: bool, what: i32) -> Res<Value> {
        let r = self.realm();
        let (proto, kind) = if map {
            (r.intrinsics.map_iterator_prototype, ObjectKind::MapIterator)
        } else {
            (r.intrinsics.set_iterator_prototype, ObjectKind::SetIterator)
        };
        let it = self.alloc_object(ObjectCell::with_internal(
            proto,
            kind,
            vec![t, Value::int(0), Value::int(what)],
        ))?;
        if let Some(o) = it.as_object() {
            self.heap.table_add_iterator(t, o);
        }
        Ok(it)
    }

    /// `forEach` of a map or set: `callback` called with each value, key
    /// and the collection, entries added meanwhile visited too.
    fn collection_for_each(&mut self, c: Call, t: Value) -> Res<Value> {
        let callback = self.arg(c, 0);
        if !self.is_callable(callback) {
            let text = self.typed_display(callback);
            return Err(self.type_error(format!("{text} is not a function")));
        }
        let this_arg = self.arg(c, 1);
        let w = self.walker(t)?;
        self.vm.temps.push(w);
        let r = (|| {
            while let Some((k, v)) = self.walk(w) {
                self.call_value(callback, this_arg, &[v, k, c.this])?;
            }
            Ok(Value::UNDEFINED)
        })();
        self.vm.temps.pop();
        r
    }

    /// AddToKeptObjects (section 9.10.3): `v` held until the job ends.
    fn keep_alive(&mut self, v: Value) {
        self.vm.kept_alive.push(v);
    }
}

pub(crate) fn construct_map(agent: &mut Agent, c: Call) -> Res<Value> {
    agent.construct_collection(c, "Map", ObjectKind::Map, |r| r.intrinsics.map_prototype)
}

pub(crate) fn construct_set(agent: &mut Agent, c: Call) -> Res<Value> {
    agent.construct_collection(c, "Set", ObjectKind::Set, |r| r.intrinsics.set_prototype)
}

pub(crate) fn construct_weak_map(agent: &mut Agent, c: Call) -> Res<Value> {
    agent.construct_collection(c, "WeakMap", ObjectKind::WeakMap, |r| {
        r.intrinsics.weak_map_prototype
    })
}

pub(crate) fn construct_weak_set(agent: &mut Agent, c: Call) -> Res<Value> {
    agent.construct_collection(c, "WeakSet", ObjectKind::WeakSet, |r| {
        r.intrinsics.weak_set_prototype
    })
}

fn map_get(agent: &mut Agent, c: Call) -> Res<Value> {
    let t = agent.table_of(c.this, ObjectKind::Map, "Map.prototype.get")?;
    Ok(agent
        .heap
        .table_get(t, agent.arg(c, 0))
        .unwrap_or(Value::UNDEFINED))
}

fn map_set(agent: &mut Agent, c: Call) -> Res<Value> {
    let t = agent.table_of(c.this, ObjectKind::Map, "Map.prototype.set")?;
    let (k, v) = (canonical(agent.arg(c, 0)), agent.arg(c, 1));
    agent.heap.table_set(t, k, v);
    Ok(c.this)
}

fn map_has(agent: &mut Agent, c: Call) -> Res<Value> {
    let t = agent.table_of(c.this, ObjectKind::Map, "Map.prototype.has")?;
    Ok(Value::bool(
        agent.heap.table_find(t, agent.arg(c, 0)).is_some(),
    ))
}

fn map_delete(agent: &mut Agent, c: Call) -> Res<Value> {
    let t = agent.table_of(c.this, ObjectKind::Map, "Map.prototype.delete")?;
    Ok(Value::bool(agent.heap.table_delete(t, agent.arg(c, 0))))
}

fn map_clear(agent: &mut Agent, c: Call) -> Res<Value> {
    let t = agent.table_of(c.this, ObjectKind::Map, "Map.prototype.clear")?;
    agent.heap.table_clear(t);
    Ok(Value::UNDEFINED)
}

fn map_size(agent: &mut Agent, c: Call) -> Res<Value> {
    let t = agent.table_of(c.this, ObjectKind::Map, "get Map.prototype.size")?;
    Ok(Value::number(agent.live_size(t)))
}

fn map_for_each(agent: &mut Agent, c: Call) -> Res<Value> {
    let t = agent.table_of(c.this, ObjectKind::Map, "Map.prototype.forEach")?;
    agent.collection_for_each(c, t)
}

fn map_entries(agent: &mut Agent, c: Call) -> Res<Value> {
    let t = agent.table_of(c.this, ObjectKind::Map, "Map.prototype.entries")?;
    agent.collection_iterator(t, true, ENTRIES)
}

fn map_keys(agent: &mut Agent, c: Call) -> Res<Value> {
    let t = agent.table_of(c.this, ObjectKind::Map, "Map.prototype.keys")?;
    agent.collection_iterator(t, true, KEYS)
}

fn map_values(agent: &mut Agent, c: Call) -> Res<Value> {
    let t = agent.table_of(c.this, ObjectKind::Map, "Map.prototype.values")?;
    agent.collection_iterator(t, true, VALUES)
}

fn set_has(agent: &mut Agent, c: Call) -> Res<Value> {
    let t = agent.table_of(c.this, ObjectKind::Set, "Set.prototype.has")?;
    Ok(Value::bool(
        agent.heap.table_find(t, agent.arg(c, 0)).is_some(),
    ))
}

fn set_add(agent: &mut Agent, c: Call) -> Res<Value> {
    let t = agent.table_of(c.this, ObjectKind::Set, "Set.prototype.add")?;
    let k = canonical(agent.arg(c, 0));
    agent.heap.table_set(t, k, k);
    Ok(c.this)
}

fn set_delete(agent: &mut Agent, c: Call) -> Res<Value> {
    let t = agent.table_of(c.this, ObjectKind::Set, "Set.prototype.delete")?;
    Ok(Value::bool(agent.heap.table_delete(t, agent.arg(c, 0))))
}

fn set_clear(agent: &mut Agent, c: Call) -> Res<Value> {
    let t = agent.table_of(c.this, ObjectKind::Set, "Set.prototype.clear")?;
    agent.heap.table_clear(t);
    Ok(Value::UNDEFINED)
}

fn set_size(agent: &mut Agent, c: Call) -> Res<Value> {
    let t = agent.table_of(c.this, ObjectKind::Set, "get Set.prototype.size")?;
    Ok(Value::number(agent.live_size(t)))
}

fn set_for_each(agent: &mut Agent, c: Call) -> Res<Value> {
    let t = agent.table_of(c.this, ObjectKind::Set, "Set.prototype.forEach")?;
    agent.collection_for_each(c, t)
}

fn set_entries(agent: &mut Agent, c: Call) -> Res<Value> {
    let t = agent.table_of(c.this, ObjectKind::Set, "Set.prototype.entries")?;
    agent.collection_iterator(t, false, ENTRIES)
}

fn set_values(agent: &mut Agent, c: Call) -> Res<Value> {
    let t = agent.table_of(c.this, ObjectKind::Set, "Set.prototype.values")?;
    agent.collection_iterator(t, false, VALUES)
}

/// `%MapIteratorPrototype%.next` and `%SetIteratorPrototype%.next`
/// (sections 24.1.5.2.1, 24.2.6.2.1).
fn iterator_next(agent: &mut Agent, c: Call, kind: ObjectKind, name: &str) -> Res<Value> {
    let what = c
        .this
        .as_object()
        .and_then(|o| agent.heap.objects.get(o))
        .filter(|cell| cell.kind() == kind)
        .and_then(|cell| cell.internal().get(2).and_then(|v| v.as_number()));
    let Some(what) = what else {
        let text = agent.display(c.this);
        return Err(agent.type_error(format!(
            "Method {name}.prototype.next called on incompatible receiver {text}"
        )));
    };
    let Some((k, v)) = agent.walk(c.this) else {
        return agent.iter_result(Value::UNDEFINED, true);
    };
    match what as i32 {
        KEYS => agent.iter_result(k, false),
        VALUES => agent.iter_result(v, false),
        _ => {
            let pair = agent.array_of(&[k, v])?;
            agent.iter_result(pair, false)
        }
    }
}

fn map_iterator_next(agent: &mut Agent, c: Call) -> Res<Value> {
    iterator_next(agent, c, ObjectKind::MapIterator, "Map Iterator")
}

fn set_iterator_next(agent: &mut Agent, c: Call) -> Res<Value> {
    iterator_next(agent, c, ObjectKind::SetIterator, "Set Iterator")
}

/// A set record (section 24.2.1.2): the other set's object, size, `has`
/// and `keys`.
#[derive(Clone, Copy)]
struct SetRecord {
    set: Value,
    size: f64,
    has: Value,
    keys: Value,
}

impl Agent {
    /// GetSetRecord (section 24.2.1.3).
    fn set_record(&mut self, obj: Value, method: &str) -> Res<SetRecord> {
        if obj.as_object().is_none() {
            return Err(self.type_error(format!("{method} argument must be an object")));
        }
        let key = self.name_key(Name::Str("size"));
        let raw = self.get_value(obj, key)?;
        let n = self.number(raw)?;
        if n.is_nan() {
            return Err(self.type_error("The .size property is NaN"));
        }
        let size = self.integer(Value::number(n))?;
        if size < 0.0 {
            let text = crate::number::number_to_string(size);
            return Err(self.range_error(format!("'{text}' is an invalid size")));
        }
        let key = self.name_key(Name::Str("has"));
        let has = self.get_value(obj, key)?;
        if !self.is_callable(has) {
            return Err(self.type_error("string \"has\" is not a function"));
        }
        self.vm.temps.push(has);
        let key = self.name_key(Name::Str("keys"));
        let keys = self.get_value(obj, key);
        self.vm.temps.pop();
        let keys = keys?;
        if !self.is_callable(keys) {
            return Err(self.type_error("string \"keys\" is not a function"));
        }
        Ok(SetRecord {
            set: obj,
            size,
            has,
            keys,
        })
    }

    /// GetIteratorFromMethod over the record's `keys` (section 24.2.1.4's
    /// GetKeysIterator).
    fn keys_iterator(&mut self, rec: SetRecord) -> Res<(Value, Value)> {
        let it = self.call_value(rec.keys, rec.set, &[])?;
        if it.as_object().is_none() {
            return Err(self.type_error("Result of the keys method is not an object"));
        }
        self.vm.temps.push(it);
        let key = self.realm().next_key;
        let next = self.get_value(it, key);
        self.vm.temps.pop();
        let next = next?;
        if !self.is_callable(next) {
            let text = self.display(next);
            return Err(self.type_error(format!("{text} is not a function")));
        }
        Ok((it, next))
    }

    fn record_has(&mut self, rec: SetRecord, v: Value) -> Res<bool> {
        let r = self.call_value(rec.has, rec.set, &[v])?;
        Ok(self.to_boolean(r))
    }

    /// A new set holding `t`'s entries (or none).
    fn set_copy(&mut self, t: Option<Value>) -> Res<(Value, Value)> {
        let proto = self.realm().intrinsics.set_prototype;
        let s = self.new_collection(proto, ObjectKind::Set, Strength::Strong)?;
        let rt = self
            .heap
            .objects
            .get(s.as_object().unwrap_or(0))
            .and_then(|c| c.internal().first().copied())
            .unwrap_or(Value::UNDEFINED);
        if let Some(t) = t {
            let mut at = 0;
            while let Some((i, k, v)) = self.heap.table_next(t, at) {
                self.heap.table_set(rt, k, v);
                at = i + 1;
            }
        }
        Ok((s, rt))
    }

    /// A set method's run: `this`'s table, the other's record, a result
    /// set rooted while `f` runs.
    fn set_method(
        &mut self,
        c: Call,
        name: &str,
        copy: bool,
        f: impl FnOnce(&mut Agent, Value, SetRecord, Value) -> Res<Value>,
    ) -> Res<Value> {
        let t = self.table_of(c.this, ObjectKind::Set, name)?;
        let rec = self.set_record(self.arg(c, 0), name)?;
        let at = self.vm.temps.len();
        self.vm.temps.extend([rec.set, rec.has, rec.keys]);
        let r = (|| {
            let (s, rt) = self.set_copy(copy.then_some(t))?;
            self.vm.temps.push(s);
            f(self, t, rec, rt).map(|v| if v.is_undefined() { s } else { v })
        })();
        self.vm.temps.truncate(at);
        r
    }

    /// Each element of the other set's keys iterator, `f` told whether to
    /// go on; the iterator closed when it stops early.
    fn each_other_key(
        &mut self,
        rec: SetRecord,
        mut f: impl FnMut(&mut Agent, Value) -> Res<bool>,
    ) -> Res<bool> {
        let (it, next) = self.keys_iterator(rec)?;
        self.vm.temps.extend([it, next]);
        let r = (|| {
            while let Some(v) = self.iterator_step(it, next)? {
                if !f(self, canonical(v))? {
                    self.iterator_close(it)?;
                    return Ok(false);
                }
            }
            Ok(true)
        })();
        let n = self.vm.temps.len();
        self.vm.temps.truncate(n.saturating_sub(2));
        r
    }

    /// Each element of `t`, live, `f` told whether to go on.
    fn each_own(
        &mut self,
        t: Value,
        mut f: impl FnMut(&mut Agent, Value) -> Res<bool>,
    ) -> Res<bool> {
        let w = self.walker(t)?;
        self.vm.temps.push(w);
        let r = (|| {
            while let Some((k, _)) = self.walk(w) {
                if !f(self, k)? {
                    return Ok(false);
                }
            }
            Ok(true)
        })();
        self.vm.temps.pop();
        r
    }
}

fn union(agent: &mut Agent, c: Call) -> Res<Value> {
    agent.set_method(c, "Set.prototype.union", true, |agent, _, rec, rt| {
        agent.each_other_key(rec, |agent, v| {
            agent.heap.table_set(rt, v, v);
            Ok(true)
        })?;
        Ok(Value::UNDEFINED)
    })
}

fn intersection(agent: &mut Agent, c: Call) -> Res<Value> {
    agent.set_method(
        c,
        "Set.prototype.intersection",
        false,
        |agent, t, rec, rt| {
            if agent.live_size(t) <= rec.size {
                agent.each_own(t, |agent, e| {
                    if agent.record_has(rec, e)? {
                        agent.heap.table_set(rt, e, e);
                    }
                    Ok(true)
                })?;
            } else {
                agent.each_other_key(rec, |agent, v| {
                    if agent.heap.table_find(t, v).is_some() {
                        agent.heap.table_set(rt, v, v);
                    }
                    Ok(true)
                })?;
            }
            Ok(Value::UNDEFINED)
        },
    )
}

fn difference(agent: &mut Agent, c: Call) -> Res<Value> {
    agent.set_method(c, "Set.prototype.difference", true, |agent, t, rec, rt| {
        if agent.live_size(t) <= rec.size {
            agent.each_own(t, |agent, e| {
                if agent.record_has(rec, e)? {
                    agent.heap.table_delete(rt, e);
                }
                Ok(true)
            })?;
        } else {
            agent.each_other_key(rec, |agent, v| {
                agent.heap.table_delete(rt, v);
                Ok(true)
            })?;
        }
        Ok(Value::UNDEFINED)
    })
}

fn symmetric_difference(agent: &mut Agent, c: Call) -> Res<Value> {
    agent.set_method(
        c,
        "Set.prototype.symmetricDifference",
        true,
        |agent, t, rec, rt| {
            agent.each_other_key(rec, |agent, v| {
                if agent.heap.table_find(t, v).is_some() {
                    agent.heap.table_delete(rt, v);
                } else {
                    agent.heap.table_set(rt, v, v);
                }
                Ok(true)
            })?;
            Ok(Value::UNDEFINED)
        },
    )
}

fn is_subset_of(agent: &mut Agent, c: Call) -> Res<Value> {
    agent.set_method(c, "Set.prototype.isSubsetOf", false, |agent, t, rec, _| {
        if agent.live_size(t) > rec.size {
            return Ok(Value::FALSE);
        }
        let all = agent.each_own(t, |agent, e| agent.record_has(rec, e))?;
        Ok(Value::bool(all))
    })
}

fn is_superset_of(agent: &mut Agent, c: Call) -> Res<Value> {
    agent.set_method(
        c,
        "Set.prototype.isSupersetOf",
        false,
        |agent, t, rec, _| {
            if agent.live_size(t) < rec.size {
                return Ok(Value::FALSE);
            }
            let all =
                agent.each_other_key(rec, |agent, v| Ok(agent.heap.table_find(t, v).is_some()))?;
            Ok(Value::bool(all))
        },
    )
}

fn is_disjoint_from(agent: &mut Agent, c: Call) -> Res<Value> {
    agent.set_method(
        c,
        "Set.prototype.isDisjointFrom",
        false,
        |agent, t, rec, _| {
            let none = if agent.live_size(t) <= rec.size {
                agent.each_own(t, |agent, e| Ok(!agent.record_has(rec, e)?))?
            } else {
                agent.each_other_key(rec, |agent, v| Ok(agent.heap.table_find(t, v).is_none()))?
            };
            Ok(Value::bool(none))
        },
    )
}

/// `Map.groupBy` (section 24.1.2.1): GroupBy with the collection's keys.
fn group_by(agent: &mut Agent, c: Call) -> Res<Value> {
    let (items, callback) = (agent.arg(c, 0), agent.arg(c, 1));
    if items.is_nullish() {
        let text = agent.typed_display(items);
        return Err(agent.type_error(crate::interp::messages::not_iterable_typed(&text)));
    }
    if !agent.is_callable(callback) {
        let text = agent.display(callback);
        return Err(agent.type_error(format!("{text} is not a function")));
    }
    let proto = agent.realm().intrinsics.map_prototype;
    let map = agent.new_collection(proto, ObjectKind::Map, Strength::Strong)?;
    let t = agent
        .heap
        .objects
        .get(map.as_object().unwrap_or(0))
        .and_then(|c| c.internal().first().copied())
        .unwrap_or(Value::UNDEFINED);
    agent.vm.temps.push(map);
    let r = (|| {
        let (it, next) = agent.get_iterator_typed(items)?;
        agent.vm.temps.extend([it, next]);
        let r = (|| {
            let mut k = 0.0;
            while let Some(v) = agent.iterator_step(it, next)? {
                let step = (|| {
                    let key =
                        agent.call_value(callback, Value::UNDEFINED, &[v, Value::number(k)])?;
                    let key = canonical(key);
                    let group = match agent.heap.table_get(t, key) {
                        Some(g) => g,
                        None => {
                            agent.vm.temps.push(key);
                            let g = agent.array_of(&[]);
                            agent.vm.temps.pop();
                            let g = g?;
                            agent.heap.table_set(t, key, g);
                            g
                        }
                    };
                    agent.array_push(group, v)
                })();
                if let Err(e) = step {
                    if matches!(e, Abrupt::Throw) {
                        agent.iterator_close_quiet(it)?;
                    }
                    return Err(e);
                }
                k += 1.0;
            }
            Ok(map)
        })();
        let n = agent.vm.temps.len();
        agent.vm.temps.truncate(n.saturating_sub(2));
        r
    })();
    agent.vm.temps.pop();
    r
}

const INVALID_WEAK_MAP_KEY: &str = "Invalid value used as weak map key";

/// `Map.prototype.getOrInsert` (the upsert proposal, which the
/// reference browser ships).
fn map_get_or_insert(agent: &mut Agent, c: Call) -> Res<Value> {
    let t = agent.table_of(c.this, ObjectKind::Map, "Map.prototype.getOrInsert")?;
    let (k, v) = (canonical(agent.arg(c, 0)), agent.arg(c, 1));
    if let Some(old) = agent.heap.table_get(t, k) {
        return Ok(old);
    }
    agent.heap.table_set(t, k, v);
    Ok(v)
}

/// `Map.prototype.getOrInsertComputed`: the callback's value set after
/// it runs, over whatever it set itself.
fn map_get_or_insert_computed(agent: &mut Agent, c: Call) -> Res<Value> {
    let t = agent.table_of(c.this, ObjectKind::Map, "Map.prototype.getOrInsertComputed")?;
    let f = agent.arg(c, 1);
    if !agent.is_callable(f) {
        let text = agent.typed_display(f);
        return Err(agent.type_error(format!("{text} is not a function")));
    }
    let k = canonical(agent.arg(c, 0));
    if let Some(old) = agent.heap.table_get(t, k) {
        return Ok(old);
    }
    let v = agent.call_value(f, Value::UNDEFINED, &[k])?;
    agent.heap.table_set(t, k, v);
    Ok(v)
}

fn weak_map_get_or_insert(agent: &mut Agent, c: Call) -> Res<Value> {
    let t = agent.table_of(c.this, ObjectKind::WeakMap, "WeakMap.prototype.getOrInsert")?;
    let (k, v) = (agent.arg(c, 0), agent.arg(c, 1));
    if !agent.can_be_held_weakly(k) {
        return Err(agent.type_error(INVALID_WEAK_MAP_KEY));
    }
    if let Some(old) = agent.heap.table_get(t, k) {
        return Ok(old);
    }
    agent.heap.table_set(t, k, v);
    Ok(v)
}

fn weak_map_get_or_insert_computed(agent: &mut Agent, c: Call) -> Res<Value> {
    let t = agent.table_of(
        c.this,
        ObjectKind::WeakMap,
        "WeakMap.prototype.getOrInsertComputed",
    )?;
    let (k, f) = (agent.arg(c, 0), agent.arg(c, 1));
    if !agent.can_be_held_weakly(k) {
        return Err(agent.type_error(INVALID_WEAK_MAP_KEY));
    }
    if !agent.is_callable(f) {
        let text = agent.typed_display(f);
        return Err(agent.type_error(format!("{text} is not a function")));
    }
    if let Some(old) = agent.heap.table_get(t, k) {
        return Ok(old);
    }
    let v = agent.call_value(f, Value::UNDEFINED, &[k])?;
    agent.heap.table_set(t, k, v);
    Ok(v)
}

fn weak_map_get(agent: &mut Agent, c: Call) -> Res<Value> {
    let t = agent.table_of(c.this, ObjectKind::WeakMap, "WeakMap.prototype.get")?;
    let k = agent.arg(c, 0);
    if !agent.can_be_held_weakly(k) {
        return Ok(Value::UNDEFINED);
    }
    Ok(agent.heap.table_get(t, k).unwrap_or(Value::UNDEFINED))
}

fn weak_map_has(agent: &mut Agent, c: Call) -> Res<Value> {
    let t = agent.table_of(c.this, ObjectKind::WeakMap, "WeakMap.prototype.has")?;
    let k = agent.arg(c, 0);
    Ok(Value::bool(
        agent.can_be_held_weakly(k) && agent.heap.table_find(t, k).is_some(),
    ))
}

fn weak_map_delete(agent: &mut Agent, c: Call) -> Res<Value> {
    let t = agent.table_of(c.this, ObjectKind::WeakMap, "WeakMap.prototype.delete")?;
    let k = agent.arg(c, 0);
    Ok(Value::bool(
        agent.can_be_held_weakly(k) && agent.heap.table_delete(t, k),
    ))
}

fn weak_map_set(agent: &mut Agent, c: Call) -> Res<Value> {
    let t = agent.table_of(c.this, ObjectKind::WeakMap, "WeakMap.prototype.set")?;
    let (k, v) = (agent.arg(c, 0), agent.arg(c, 1));
    if !agent.can_be_held_weakly(k) {
        return Err(agent.type_error(INVALID_WEAK_MAP_KEY));
    }
    agent.heap.table_set(t, k, v);
    Ok(c.this)
}

fn weak_set_has(agent: &mut Agent, c: Call) -> Res<Value> {
    let t = agent.table_of(c.this, ObjectKind::WeakSet, "WeakSet.prototype.has")?;
    let k = agent.arg(c, 0);
    Ok(Value::bool(
        agent.can_be_held_weakly(k) && agent.heap.table_find(t, k).is_some(),
    ))
}

fn weak_set_delete(agent: &mut Agent, c: Call) -> Res<Value> {
    let t = agent.table_of(c.this, ObjectKind::WeakSet, "WeakSet.prototype.delete")?;
    let k = agent.arg(c, 0);
    Ok(Value::bool(
        agent.can_be_held_weakly(k) && agent.heap.table_delete(t, k),
    ))
}

fn weak_set_add(agent: &mut Agent, c: Call) -> Res<Value> {
    let t = agent.table_of(c.this, ObjectKind::WeakSet, "WeakSet.prototype.add")?;
    let k = agent.arg(c, 0);
    if !agent.can_be_held_weakly(k) {
        return Err(agent.type_error("Invalid value used in weak set"));
    }
    agent.heap.table_set(t, k, Value::TRUE);
    Ok(c.this)
}

/// `WeakRef(target)` (section 26.1.1.1).
pub(crate) fn construct_weak_ref(agent: &mut Agent, c: Call) -> Res<Value> {
    if c.new_target.is_undefined() {
        return Err(agent.type_error("Constructor WeakRef requires 'new'"));
    }
    let target = agent.arg(c, 0);
    if !agent.can_be_held_weakly(target) {
        return Err(agent.type_error("WeakRef: invalid target"));
    }
    let proto = agent.prototype_from(c.new_target, |r| r.intrinsics.weak_ref_prototype)?;
    let o = agent.new_collection(proto, ObjectKind::WeakRef, Strength::Weak)?;
    let t = agent
        .slots_of_value(o)
        .first()
        .copied()
        .unwrap_or(Value::UNDEFINED);
    agent.heap.table_set(t, target, Value::TRUE);
    agent.keep_alive(target);
    Ok(o)
}

fn weak_ref_deref(agent: &mut Agent, c: Call) -> Res<Value> {
    let t = agent.table_of(c.this, ObjectKind::WeakRef, "WeakRef.prototype.deref")?;
    match agent.heap.table_next(t, 0) {
        Some((_, target, _)) => {
            agent.keep_alive(target);
            Ok(target)
        }
        None => Ok(Value::UNDEFINED),
    }
}

/// `FinalizationRegistry(cleanupCallback)` (section 26.2.1.1).
pub(crate) fn construct_registry(agent: &mut Agent, c: Call) -> Res<Value> {
    if c.new_target.is_undefined() {
        return Err(agent.type_error("Constructor FinalizationRegistry requires 'new'"));
    }
    let cleanup = agent.arg(c, 0);
    if !agent.is_callable(cleanup) {
        return Err(agent.type_error("FinalizationRegistry: cleanup must be callable"));
    }
    let proto = agent.prototype_from(c.new_target, |r| r.intrinsics.registry_prototype)?;
    let o = agent.new_collection(proto, ObjectKind::FinalizationRegistry, Strength::Registry)?;
    let t = agent
        .slots_of_value(o)
        .first()
        .copied()
        .unwrap_or(Value::UNDEFINED);
    if let Some((_, i)) = t.as_reference() {
        agent.heap.with_internal(i, |cell| {
            if let InternalCell::Table(table) = cell {
                table.cleanup = cleanup;
            }
        });
    }
    Ok(o)
}

fn register(agent: &mut Agent, c: Call) -> Res<Value> {
    let t = agent.table_of(
        c.this,
        ObjectKind::FinalizationRegistry,
        "FinalizationRegistry.prototype.register",
    )?;
    let (target, held, token) = (agent.arg(c, 0), agent.arg(c, 1), agent.arg(c, 2));
    if !agent.can_be_held_weakly(target) {
        return Err(agent.type_error("FinalizationRegistry.prototype.register: invalid target"));
    }
    if crate::object::same_value(&agent.heap, target, held) {
        return Err(agent.type_error(
            "FinalizationRegistry.prototype.register: target and holdings must not be same",
        ));
    }
    if !token.is_undefined() && !agent.can_be_held_weakly(token) {
        let text = agent.display(token);
        return Err(agent.type_error(format!("Invalid unregisterToken ('{text}')")));
    }
    agent.heap.table_register(t, target, held, token);
    Ok(Value::UNDEFINED)
}

fn unregister(agent: &mut Agent, c: Call) -> Res<Value> {
    let t = agent.table_of(
        c.this,
        ObjectKind::FinalizationRegistry,
        "FinalizationRegistry.prototype.unregister",
    )?;
    let token = agent.arg(c, 0);
    if !agent.can_be_held_weakly(token) {
        let text = agent.display(token);
        return Err(agent.type_error(format!("Invalid unregisterToken ('{text}')")));
    }
    Ok(Value::bool(agent.heap.table_unregister(t, token)))
}

impl Agent {
    /// A registry's cleanup job (section 9.13's
    /// HostEnqueueFinalizationRegistryCleanupJob): its callback called
    /// with each held value whose target died.
    pub(crate) fn run_cleanup(&mut self, t: Value) -> Res<()> {
        let Some((_, i)) = t.as_reference() else {
            return Ok(());
        };
        let taken = self.heap.with_internal(i, |cell| match cell {
            InternalCell::Table(table) => {
                table.cleanup_queued = false;
                Some((table.cleanup, std::mem::take(&mut table.cleared)))
            }
            _ => None,
        });
        let Some(Some((callback, held))) = taken else {
            return Ok(());
        };
        let at = self.vm.temps.len();
        self.vm.temps.extend_from_slice(&held);
        let r = (|| {
            for h in held {
                self.call_value(callback, Value::UNDEFINED, &[h])?;
            }
            Ok(())
        })();
        self.vm.temps.truncate(at);
        r
    }
}
