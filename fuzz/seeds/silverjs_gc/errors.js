function thrower() { null.x; }
try { thrower(); } catch (e) { e.stack; e.message; e instanceof TypeError; }
Error.stackTraceLimit = 3; Error.prepareStackTrace = function (e, s) { return s.map(String).join('|'); };
var r = new RangeError('r', { cause: 1 }).stack; Error.prepareStackTrace = undefined;
var o = {}; Error.captureStackTrace(o); o.stack;
try { (function deep() { deep(); })(); } catch (e) { r += e.message; }
try { throw { toString: function () { throw 1; } }; } finally { r; }
