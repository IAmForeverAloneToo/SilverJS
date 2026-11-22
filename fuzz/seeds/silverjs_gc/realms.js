3var other = createRealm();
other.eval('var shared = []; function f() { return this; }');
var a = other.Array.of(1, 2); a.constructor === other.Array;
var r = [a.concat() instanceof Array, other.f() === other, Reflect.construct(Array, [], other.Function) instanceof other.Array];
try { other.Function.prototype.call.call(1); } catch (e) { r.push(e instanceof other.TypeError); }
gc(); other.createRealm().gc(); r.concat(other.shared);
