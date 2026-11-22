2var out = [];
outer: for (var i = 0; i < 5; i++) { switch (i % 3) { case 0: continue outer; case 1: out.push(i); break; default: break outer; } }
do { out.push('d'); } while (false);
with ({ w: 1 }) { out.push(w); }
label: { try { break label; } finally { out.push('f'); } }
var t = typeof undeclared + void 0 + (1, 2) + (out ? 'y' : 'n'); out.join() + t;
