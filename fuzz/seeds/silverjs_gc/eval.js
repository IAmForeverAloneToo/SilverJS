0var x = 'g';
function f() { var x = 'l'; eval('var y = x + 1; function h() { return y; }'); return h() + (0, eval)('x'); }
f() + new Function('a', 'b', 'return a + b')(1, 2) + eval('let z = 3; z');
(function () { 'use strict'; eval('var w = 1'); return typeof w; })();
