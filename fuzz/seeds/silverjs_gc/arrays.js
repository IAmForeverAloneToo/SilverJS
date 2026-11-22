var a = [3, 1, 2]; a.length = 10; a[20] = 'x';
a.sort(function (x, y) { return y - x; }).reverse().fill(0, 5, 7);
var b = a.map(String).filter(Boolean).concat([[1, [2]]].flat(Infinity));
b.splice(1, 2, 'p', 'q'); b.indexOf('q'); b.lastIndexOf(undefined); b.includes(NaN);
Array.from({ length: 3 }, function (v, i) { return i * i; }).join('-') + Array.of(1, 2) + b.slice(-3);
