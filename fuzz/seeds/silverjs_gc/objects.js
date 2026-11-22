var o = { a: 1, get b() { return this.a + 1; }, set c(v) { this.a = v; } };
o.c = 5; Object.defineProperty(o, 'd', { value: [1, , 3], enumerable: false });
var k = Object.keys(o).concat(Object.getOwnPropertyNames(o.d), Reflect.ownKeys(Symbol));
Object.freeze(o); delete o.a; for (var p in o) k.push(p);
JSON = Object.assign(Object.create(null), o, { e: Symbol('s') });
[k.join(), Object.getOwnPropertyDescriptor(o, 'b').get.name, o instanceof Object];
