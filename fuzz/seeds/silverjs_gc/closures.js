1function counter() { var n = 0; return { inc: function () { return ++n; }, get: () => n }; }
var c = counter(); c.inc(); c.inc();
var fs = []; for (let i = 0; i < 4; i++) fs.push(function () { return i; });
(function f(depth) { return depth ? f(depth - 1) : arguments.callee.name; })(50);
var bound = function (a, b) { return this.x + a + b; }.bind({ x: 1 }, 2);
[c.get(), fs.map(function (g) { return g(); }), bound(3), new (function F() { this.y = new.target === F; })().y];
