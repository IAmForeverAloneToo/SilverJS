function* g(a = 1, ...r) { yield* r; }
async function f() { for await (const x of y) await x; }
const h = async (a, { b }) => ({ a, b });
(function () { 'use strict'; return new.target; })();
