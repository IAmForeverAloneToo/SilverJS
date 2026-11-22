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
}

impl ErrorType {
    pub(crate) const ALL: [ErrorType; 7] = [
        ErrorType::Error,
        ErrorType::TypeError,
        ErrorType::RangeError,
        ErrorType::ReferenceError,
        ErrorType::SyntaxError,
        ErrorType::EvalError,
        ErrorType::UriError,
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
        }
    }
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
