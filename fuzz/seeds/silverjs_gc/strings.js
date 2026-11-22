0var s = ('abc' + String.fromCharCode(0xe9, 0xd83d, 0xde00)).repeat(3);
s = s.padStart(30, '-').toUpperCase().split('').reverse().join('');
s.replace('A', function (m, i) { return i; }).replaceAll('B', '$&$&').trim().at(-1);
[s.length, s.charCodeAt(4), s.codePointAt(5), String.fromCharCode(65, 66), s.normalize === undefined,
 encodeURIComponent(s), decodeURI('%41'), escape('a b'), parseInt('0x1f'), parseFloat('3.5e1x')];
