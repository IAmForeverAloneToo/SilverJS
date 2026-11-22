var n = [0.1 + 0.2, -0, 1 / 3, 2 ** 53 + 1, 1e21, 5e-324, NaN, -Infinity];
n.map(function (x) { return [x.toFixed(5), x.toPrecision(3), x.toExponential(2), x.toString(36)]; });
Math.max.apply(null, n) + Math.hypot(3, 4) + Math.round(-0.5) + Math.clz32(1) + Math.imul(3, 7);
Number.isInteger(5.0) && Number.isSafeInteger(2 ** 53) || Object.is(-0, 0) | ~~3.7 >>> 1 << 2;
