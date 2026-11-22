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
