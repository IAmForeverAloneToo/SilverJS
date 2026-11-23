//! The messages of the errors the engine throws, worded as the reference
//! browser words them (docs/design.md section 1.11; the observed texts
//! are `docs/chrome-surface.md`'s). One table, so that the wording is
//! reviewed in one place; each text is pinned by a test of Chrome's
//! observed output.

/// The error constructors, by the prototype a thrown error gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ErrorType {
    Error,
    TypeError,
    RangeError,
    ReferenceError,
    SyntaxError,
    EvalError,
    UriError,
    AggregateError,
}

impl ErrorType {
    pub(crate) const ALL: [ErrorType; 8] = [
        ErrorType::Error,
        ErrorType::TypeError,
        ErrorType::RangeError,
        ErrorType::ReferenceError,
        ErrorType::SyntaxError,
        ErrorType::EvalError,
        ErrorType::UriError,
        ErrorType::AggregateError,
    ];

    pub(crate) fn name(self) -> &'static str {
        match self {
            ErrorType::Error => "Error",
            ErrorType::TypeError => "TypeError",
            ErrorType::RangeError => "RangeError",
            ErrorType::ReferenceError => "ReferenceError",
            ErrorType::SyntaxError => "SyntaxError",
            ErrorType::EvalError => "EvalError",
            ErrorType::UriError => "URIError",
            ErrorType::AggregateError => "AggregateError",
        }
    }
}

/// `Chaining cycle detected for promise #<Promise>`.
pub(crate) fn promise_cycle() -> String {
    "Chaining cycle detected for promise #<Promise>".to_owned()
}

/// NewPromiseCapability's check of the functions it was given.
pub(crate) fn promise_resolve_not_function(_what: &str) -> String {
    "Promise resolve or reject function is not callable".to_owned()
}

pub(crate) fn promise_reject_not_function(_what: &str) -> String {
    "Promise resolve or reject function is not callable".to_owned()
}

pub(crate) const CONSTRUCTOR_NOT_OBJECT: &str = "The .constructor property is not an object";
pub(crate) const PROMISE_WITHOUT_NEW: &str = "Promise constructor cannot be invoked without 'new'";
pub(crate) const PROMISE_EXECUTOR_TWICE: &str =
    "Promise executor has already been invoked with non-undefined arguments";
pub(crate) const PROMISE_RESOLVE_NON_OBJECT: &str = "PromiseResolve called on non-object";
pub(crate) const ALL_REJECTED: &str = "All promises were rejected";

/// `Promise resolver 1 is not a function`.
pub(crate) fn promise_resolver(what: &str) -> String {
    format!("Promise resolver {what} is not a function")
}

/// `Method Promise.prototype.then called on incompatible receiver
/// #<Object>`.
pub(crate) fn incompatible_receiver(method: &str, what: &str) -> String {
    format!("Method {method} called on incompatible receiver {what}")
}

/// `Cannot read properties of null (reading 'length')`.
pub(crate) fn read_properties(of: &str, key: &str) -> String {
    format!("Cannot read properties of {of} (reading '{key}')")
}

/// `Cannot set properties of undefined (setting 'x')`.
pub(crate) fn set_properties(of: &str, key: &str) -> String {
    format!("Cannot set properties of {of} (setting '{key}')")
}

/// `o.f is not a function`: the callee's source text as written.
pub(crate) fn not_a_function(callee: &str) -> String {
    format!("{callee} is not a function")
}

/// `F is not a constructor`.
pub(crate) fn not_a_constructor(callee: &str) -> String {
    format!("{callee} is not a constructor")
}

pub(crate) const CONST_ASSIGNMENT: &str = "Assignment to constant variable.";

/// `Cannot destructure property 'a' of 'o.x' as it is undefined.`, or
/// without a property where the pattern names none first.
pub(crate) fn cannot_destructure(name: &str, what: &str, value: &str) -> String {
    if name.is_empty() {
        format!("Cannot destructure '{what}' as it is {value}.")
    } else {
        format!("Cannot destructure property '{name}' of '{what}' as it is {value}.")
    }
}

/// `x is not iterable`: the iterable's source text as written.
pub(crate) fn not_iterable(what: &str) -> String {
    format!("{what} is not iterable")
}

pub(crate) const ITERATOR_NOT_OBJECT: &str =
    "Result of the Symbol.iterator method is not an object";

/// `Class extends value 1 is not a constructor or null`.
pub(crate) fn extends_not_constructor(what: &str) -> String {
    format!("Class extends value {what} is not a constructor or null")
}

/// `Class extends value does not have valid prototype property 1`.
pub(crate) fn extends_bad_prototype(what: &str) -> String {
    format!("Class extends value does not have valid prototype property {what}")
}

/// `Super constructor max of X is not a constructor` (`of anonymous
/// class` for a class with no name).
pub(crate) fn super_not_constructor(what: &str, class: &str) -> String {
    format!("Super constructor {what} of {class} is not a constructor")
}

/// `Class constructor A cannot be invoked without 'new'`.
pub(crate) fn class_without_new(name: &str) -> String {
    format!("Class constructor {name} cannot be invoked without 'new'")
}

/// `Cannot redefine property: x`.
pub(crate) fn cannot_redefine(key: &str) -> String {
    format!("Cannot redefine property: {key}")
}

pub(crate) const THIS_BEFORE_SUPER: &str = "Must call super constructor in derived class before accessing 'this' or returning from derived constructor";

/// `Cannot read private member #p from an object whose class did not
/// declare it`.
pub(crate) fn private_read(name: &str) -> String {
    format!("Cannot read private member {name} from an object whose class did not declare it")
}

/// `Cannot write private member #p to an object whose class did not
/// declare it`.
pub(crate) fn private_write(name: &str) -> String {
    format!("Cannot write private member {name} to an object whose class did not declare it")
}

/// `Receiver must be an instance of class X` (a private method's or
/// accessor's), or `Receiver must be class X` (a static one's).
pub(crate) fn private_receiver(class: &str, is_static: bool) -> String {
    if is_static {
        format!("Receiver must be class {class}")
    } else {
        format!("Receiver must be an instance of class {class}")
    }
}

/// `'#g' was defined without a setter`.
pub(crate) fn private_no_setter(name: &str) -> String {
    format!("'{name}' was defined without a setter")
}

/// `'#s' was defined without a getter`.
pub(crate) fn private_no_getter(name: &str) -> String {
    format!("'{name}' was defined without a getter")
}

/// `Private method '#m' is not writable`.
pub(crate) fn private_method_write(name: &str) -> String {
    format!("Private method '{name}' is not writable")
}

/// `Cannot initialize #p twice on the same object`.
pub(crate) fn private_twice(name: &str) -> String {
    format!("Cannot initialize {name} twice on the same object")
}

/// `Cannot initialize private methods of class X twice on the same
/// object`.
pub(crate) fn brand_twice(class: &str) -> String {
    format!("Cannot initialize private methods of class {class} twice on the same object")
}

/// `Cannot use 'in' operator to search for '#p' in 1`.
pub(crate) fn private_in(name: &str, what: &str) -> String {
    format!("Cannot use 'in' operator to search for '{name}' in {what}")
}

/// `Cannot define property f, object is not extensible`.
pub(crate) fn field_not_extensible(key: &str) -> String {
    format!("Cannot define property {key}, object is not extensible")
}

pub(crate) const PROXY_WITHOUT_NEW: &str = "Constructor Proxy requires 'new'";
pub(crate) const PROXY_NON_OBJECT: &str =
    "Cannot create proxy with a non-object as target or handler";
pub(crate) const PROXY_PROTO_BAD: &str =
    "'getPrototypeOf' on proxy: trap returned neither object nor null";
pub(crate) const PROXY_PROTO_INVARIANT: &str = "'getPrototypeOf' on proxy: proxy target is non-extensible but the trap did not return its actual prototype";
pub(crate) const PROXY_SET_PROTO_INVARIANT: &str = "'setPrototypeOf' on proxy: trap returned truish for setting a new prototype on the non-extensible proxy target";
pub(crate) const PROXY_PREVENT_INVARIANT: &str =
    "'preventExtensions' on proxy: trap returned truish but the proxy target is extensible";
pub(crate) const PROXY_OWN_KEYS_NOT_OBJECT: &str = "CreateListFromArrayLike called on non-object";
pub(crate) const PROXY_OWN_KEYS_EXTRA: &str =
    "'ownKeys' on proxy: trap returned extra keys but proxy target is non-extensible";
pub(crate) const PROXY_OWN_KEYS_DUPLICATE: &str =
    "'ownKeys' on proxy: trap returned duplicate entries";
pub(crate) const PROXY_SET_PROTO_FALSE: &str =
    "'setPrototypeOf' on proxy: trap returned falsish for property 'undefined'";
pub(crate) const PROXY_PREVENT_FALSE: &str = "'preventExtensions' on proxy: trap returned falsish";

/// A proxy's `construct` trap returning a primitive.
pub(crate) fn proxy_construct_not_object(what: &str) -> String {
    format!("'construct' on proxy: trap returned non-object ('{what}')")
}

/// A proxy's `deleteProperty` trap refusing in strict code.
pub(crate) fn proxy_delete_false(key: &str) -> String {
    format!("'deleteProperty' on proxy: trap returned falsish for property '{key}'")
}

pub(crate) const CANNOT_PREVENT_EXTENSIONS: &str = "Cannot prevent extensions";

/// A proxy's `defineProperty` trap refusing.
pub(crate) fn proxy_define_false(key: &str) -> String {
    format!("'defineProperty' on proxy: trap returned falsish for property '{key}'")
}

/// A proxy's `set` trap refusing in strict code.
pub(crate) fn proxy_set_false(key: &str) -> String {
    format!("'set' on proxy: trap returned falsish for property '{key}'")
}

/// `Cannot perform 'get' on a proxy that has been revoked`.
pub(crate) fn proxy_revoked(what: &str) -> String {
    format!("Cannot perform '{what}' on a proxy that has been revoked")
}

pub(crate) fn proxy_trap_not_function(trap: &str, value: &str, handler: &str) -> String {
    format!("'{value}' returned for property '{trap}' of object '{handler}' is not a function")
}

pub(crate) fn proxy_extensible_invariant(expected: bool) -> String {
    format!(
        "'isExtensible' on proxy: trap result does not reflect extensibility of proxy target (which is '{expected}')"
    )
}

pub(crate) fn proxy_gopd_bad(key: &str) -> String {
    format!(
        "'getOwnPropertyDescriptor' on proxy: trap returned neither object nor undefined for property '{key}'"
    )
}

pub(crate) fn proxy_gopd_hide_nonconfigurable(key: &str) -> String {
    format!(
        "'getOwnPropertyDescriptor' on proxy: trap returned undefined for property '{key}' which is non-configurable in the proxy target"
    )
}

pub(crate) fn proxy_gopd_hide_nonextensible(key: &str) -> String {
    format!(
        "'getOwnPropertyDescriptor' on proxy: trap returned undefined for property '{key}' which exists in the non-extensible proxy target"
    )
}

pub(crate) fn proxy_gopd_incompatible(key: &str) -> String {
    format!(
        "'getOwnPropertyDescriptor' on proxy: trap returned descriptor for property '{key}' that is incompatible with the existing property in the proxy target"
    )
}

pub(crate) fn proxy_gopd_nonconfigurable(key: &str) -> String {
    format!(
        "'getOwnPropertyDescriptor' on proxy: trap reported non-configurability for property '{key}' which is either non-existent or configurable in the proxy target"
    )
}

pub(crate) fn proxy_define_nonextensible(key: &str) -> String {
    format!(
        "'defineProperty' on proxy: trap returned truish for adding property '{key}'  to the non-extensible proxy target"
    )
}

pub(crate) fn proxy_define_nonconfigurable(key: &str) -> String {
    format!(
        "'defineProperty' on proxy: trap returned truish for defining non-configurable property '{key}' which is either non-existent or configurable in the proxy target"
    )
}

pub(crate) fn proxy_define_incompatible(key: &str) -> String {
    format!(
        "'defineProperty' on proxy: trap returned truish for adding property '{key}'  that is incompatible with the existing property in the proxy target"
    )
}

pub(crate) fn proxy_has_nonconfigurable(key: &str) -> String {
    format!(
        "'has' on proxy: trap returned falsish for property '{key}' which exists in the proxy target as non-configurable"
    )
}

pub(crate) fn proxy_has_nonextensible(key: &str) -> String {
    format!(
        "'has' on proxy: trap returned falsish for property '{key}' but the proxy target is not extensible"
    )
}

pub(crate) fn proxy_get_data(key: &str, want: &str, got: &str) -> String {
    format!(
        "'get' on proxy: property '{key}' is a read-only and non-configurable data property on the proxy target but the proxy did not return its actual value (expected '{want}' but got '{got}')"
    )
}

pub(crate) fn proxy_get_accessor(key: &str, got: &str) -> String {
    format!(
        "'get' on proxy: property '{key}' is a non-configurable accessor property on the proxy target and does not have a getter function, but the trap did not return 'undefined' (got '{got}')"
    )
}

pub(crate) fn proxy_set_data(key: &str) -> String {
    format!(
        "'set' on proxy: trap returned truish for property '{key}' which exists in the proxy target as a non-configurable and non-writable data property with a different value"
    )
}

pub(crate) fn proxy_set_accessor(key: &str) -> String {
    format!(
        "'set' on proxy: trap returned truish for property '{key}' which exists in the proxy target as a non-configurable and non-writable accessor property without a setter"
    )
}

pub(crate) fn proxy_delete_nonconfigurable(key: &str) -> String {
    format!(
        "'deleteProperty' on proxy: trap returned truish for property '{key}' which is non-configurable in the proxy target"
    )
}

pub(crate) fn proxy_delete_nonextensible(key: &str) -> String {
    format!(
        "'deleteProperty' on proxy: trap returned truish for property '{key}' but the proxy target is non-extensible"
    )
}

pub(crate) fn proxy_own_keys_bad(what: &str) -> String {
    format!("{what} is not a valid property name")
}

pub(crate) fn proxy_own_keys_missing(key: &str) -> String {
    format!("'ownKeys' on proxy: trap result did not include '{key}'")
}

pub(crate) fn proxy_own_keys_missing_nonextensible(key: &str) -> String {
    format!("'ownKeys' on proxy: trap result did not include '{key}'")
}

pub(crate) const SUPER_DELETE: &str = "Unsupported reference to 'super'";
pub(crate) const SUPER_TWICE: &str = "Super constructor may only be called once";

pub(crate) const DERIVED_RETURN: &str = "Derived constructors may only return object or undefined";

pub(crate) const GENERATOR_RUNNING: &str = "Generator is already running";

pub(crate) const DELEGATE_NO_THROW: &str = "The iterator does not provide a 'throw' method.";

/// `Method [Generator].prototype.next called on incompatible receiver
/// #<Object>`.
pub(crate) fn generator_incompatible(method: &str, what: &str) -> String {
    format!("Method [Generator].prototype.{method} called on incompatible receiver {what}")
}

/// `Method [AsyncGenerator].prototype.next called on incompatible
/// receiver #<Object>`.
pub(crate) fn async_generator_incompatible(method: &str, what: &str) -> String {
    format!("Method [AsyncGenerator].prototype.{method} called on incompatible receiver {what}")
}

/// `1 is not async iterable`: a `for await`'s value, by its source.
pub(crate) fn not_async_iterable(what: &str) -> String {
    format!("{what} is not async iterable")
}

pub(crate) const ASYNC_ITERATOR_NOT_OBJECT: &str =
    "Result of the Symbol.asyncIterator method is not an object";

/// `number 1 is not iterable (cannot read property
/// Symbol(Symbol.iterator))`: a `yield*`'s value.
pub(crate) fn not_iterable_typed(what: &str) -> String {
    format!("{what} is not iterable (cannot read property Symbol(Symbol.iterator))")
}

pub(crate) const SPREAD_NOT_ITERABLE: &str =
    "Spread syntax requires ...iterable[Symbol.iterator] to be a function";

/// `Iterator result 1 is not an object`.
pub(crate) fn iterator_result_not_object(what: &str) -> String {
    format!("Iterator result {what} is not an object")
}

/// `Cannot access 'x' before initialization`.
pub(crate) fn before_initialization(name: &str) -> String {
    format!("Cannot access '{name}' before initialization")
}

/// `x is not defined`.
pub(crate) fn not_defined(name: &str) -> String {
    format!("{name} is not defined")
}

pub(crate) const SYMBOL_TO_STRING: &str = "Cannot convert a Symbol value to a string";
pub(crate) const SYMBOL_TO_NUMBER: &str = "Cannot convert a Symbol value to a number";
pub(crate) const STACK_OVERFLOW: &str = "Maximum call stack size exceeded";
pub(crate) const INVALID_ASSIGNMENT: &str = "Invalid left-hand side in assignment";
pub(crate) const NULLISH_TO_OBJECT: &str = "Cannot convert undefined or null to object";
pub(crate) const OBJECT_TO_PRIMITIVE: &str = "Cannot convert object to primitive value";
pub(crate) const OUT_OF_MEMORY: &str = "Out of memory";
pub(crate) const INSTANCEOF_NOT_CALLABLE: &str = "Right-hand side of 'instanceof' is not callable";
pub(crate) const INSTANCEOF_NOT_OBJECT: &str = "Right-hand side of 'instanceof' is not an object";

/// `Cannot use 'in' operator to search for 'x' in 1`.
pub(crate) fn in_not_object(key: &str, value: &str) -> String {
    format!("Cannot use 'in' operator to search for '{key}' in {value}")
}

/// `Cannot assign to read only property 'x' of object '#<Object>'`.
pub(crate) fn read_only(key: &str, object: &str) -> String {
    format!("Cannot assign to read only property '{key}' of object '{object}'")
}

/// `Cannot create property 'x' on string 'abc'`.
pub(crate) fn create_on_primitive(key: &str, kind: &str, value: &str) -> String {
    format!("Cannot create property '{key}' on {kind} '{value}'")
}

/// `Cannot delete property 'x' of #<Object>`.
pub(crate) fn cannot_delete(key: &str, object: &str) -> String {
    format!("Cannot delete property '{key}' of {object}")
}

/// `Identifier 'x' has already been declared`, for a script's global
/// declarations that clash with an earlier script's.
pub(crate) fn already_declared(name: &str) -> String {
    format!("Identifier '{name}' has already been declared")
}

/// What J2 does not run yet, thrown where the program reaches it.
pub(crate) fn not_supported(what: &str) -> String {
    format!("{what} is not supported yet")
}
