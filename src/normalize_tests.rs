//! Unicode normalization and the Unicode version (J4.4d):
//! `String.prototype.normalize` over the tables of UCD 17.0.0, the
//! version the reference browser ships, which every generated table and
//! the case mapping share (the J4.4d probes, Chrome 158).

use crate::agent::{Agent, AgentOptions, Outcome};

fn run(src: &str) -> String {
    let mut agent = Agent::new(AgentOptions::default());
    agent.scope(|scope| match scope.run_script_named(src, "n.js") {
        Ok(Outcome::Value(v)) => scope.to_text(v).unwrap_or_default(),
        Ok(Outcome::Threw(e)) => format!("throw {}", scope.to_text(e).unwrap_or_default()),
        Ok(Outcome::Terminated) => "terminated".into(),
        Err(e) => format!("compile error: {e}"),
    })
}

#[test]
fn the_unicode_version_is_one_everywhere() {
    // The reference browser's (the J4.4d probe): every generated table,
    // and the standard library's, whose case mapping String uses.
    assert_eq!(crate::UNICODE_VERSION, "17.0.0");
    let (a, b, c) = std::char::UNICODE_VERSION;
    assert_eq!(format!("{a}.{b}.{c}"), crate::UNICODE_VERSION);
    let database = format!("Database {}.", crate::UNICODE_VERSION);
    assert!(include_str!("normalize_tables.rs").contains(&database));
    assert!(include_str!("unicode_tables.rs").contains(&database));
    let ucd = format!("UCD {}'s", crate::UNICODE_VERSION);
    assert!(include_str!("regexp/properties.rs").contains(&ucd));
}

/// The J4.4d probes' facts: identifiers, case mapping and decomposition
/// of code points new in 16.0, 17.0 and 18.0, normalize's four forms and
/// its messages.
const PROBE: &str = r#"var points = [
  ['16.0 Todhri A', 0x105C0], ['16.0 Garay letter', 0x10D4A], ['17.0 Sidetic N01', 0x10940],
  ['17.0 Tolong Siki I', 0x11DB0], ['17.0 Beria Erfe small arkab', 0x16EBB], ['17.0 Tai Yo low ko', 0x1E6C0],
  ['18.0 Jurchen 18E00', 0x18E00], ['18.0 Armenian small eh', 0x0558], ['18.0 Latin closed omega', 0xA7DD],
  ['18.0 Latin script R', 0xAB6C], ['18.0 Latin dezh digraph curl', 0x1DF2B]
];
function hex(s) { return Array.from(s).map(function (c) { return c.codePointAt(0).toString(16); }).join('+'); }
var out = points.map(function (p) {
  var s = String.fromCodePoint(p[1]);
  var ident;
  try { ident = typeof new Function('var ' + s + ' = 1; return ' + s + ';'); } catch (e) { ident = e.constructor.name; }
  return p[0] + ' id=' + ident + ' up=' + s.toUpperCase().codePointAt(0).toString(16) +
    ' lo=' + s.toLowerCase().codePointAt(0).toString(16) + ' nfd=' + hex(s.normalize('NFD'));
});
out.push('normalize ' + ['A\u030A', '\u212B', '\u1E9B\u0323', '\uFB01', '\u0958', '\uAC00\u11A8'].map(function (s) {
  return ['NFC', 'NFD', 'NFKC', 'NFKD'].map(function (f) { return hex(s.normalize(f)); }).join('/');
}).join(' '));
function err(label, fn) { try { out.push(label + ': no throw ' + fn()); } catch (e) { out.push(label + ': ' + e.constructor.name + ': ' + e.message); } }
err('bad form', function () { return 'a'.normalize('nfc'); });
err('null this', function () { return String.prototype.normalize.call(null); });
err('undefined form', function () { return ('a' + String.fromCharCode(0x301)).normalize(undefined).length; });
err('lone surrogate', function () {
  var s = String.fromCharCode(0xD800, 0x301, 0x61, 0xDC00, 0x41, 0x30A);
  return Array.from(s.normalize('NFC')).map(function (c) { return c.charCodeAt(0).toString(16); }).join('+');
});
out.join('\n')"#;

#[test]
fn normalize_and_the_version_read_as_the_reference_browser_gives_them() {
    let got = run(PROBE);
    let want = r#"16.0 Todhri A id=function up=105c0 lo=105c0 nfd=105c0
16.0 Garay letter id=function up=10d4a lo=10d4a nfd=10d4a
17.0 Sidetic N01 id=function up=10940 lo=10940 nfd=10940
17.0 Tolong Siki I id=function up=11db0 lo=11db0 nfd=11db0
17.0 Beria Erfe small arkab id=function up=16ea0 lo=16ebb nfd=16ebb
17.0 Tai Yo low ko id=function up=1e6c0 lo=1e6c0 nfd=1e6c0
18.0 Jurchen 18E00 id=SyntaxError up=18e00 lo=18e00 nfd=18e00
18.0 Armenian small eh id=SyntaxError up=558 lo=558 nfd=558
18.0 Latin closed omega id=SyntaxError up=a7dd lo=a7dd nfd=a7dd
18.0 Latin script R id=SyntaxError up=ab6c lo=ab6c nfd=ab6c
18.0 Latin dezh digraph curl id=SyntaxError up=1df2b lo=1df2b nfd=1df2b
normalize c5/41+30a/c5/41+30a c5/41+30a/c5/41+30a 1e9b+323/17f+323+307/1e69/73+323+307 fb01/fb01/66+69/66+69 915+93c/915+93c/915+93c/915+93c ac01/1100+1161+11a8/ac01/1100+1161+11a8
bad form: RangeError: The normalization form should be one of NFC, NFD, NFKC, NFKD.
null this: TypeError: String.prototype.normalize called on null or undefined
undefined form: no throw 1
lone surrogate: no throw d800+301+61+dc00+c5"#;
    for (g, w) in got.lines().zip(want.lines()) {
        assert_eq!(g, w);
    }
    assert_eq!(got.lines().count(), want.lines().count(), "{got}");
}

/// Every 97th line of UCD 17.0.0's NormalizationTest.txt (c1 to c5, in
/// hex), whose invariants the whole file's check holds for each line
/// (the J4.4d check: all 4,780,592, as the reference browser does too).
const CONFORMANCE: &[[&str; 5]] = &[
    ["1E0A", "1E0A", "0044 0307", "1E0A", "0044 0307"],
    ["00ED", "00ED", "0069 0301", "00ED", "0069 0301"],
    ["015E", "015E", "0053 0327", "015E", "0053 0327"],
    ["0206", "0206", "0045 0311", "0206", "0045 0311"],
    ["03F2", "03F2", "03F2", "03C2", "03C2"],
    ["0BCA", "0BCA", "0BC6 0BBE", "0BCA", "0BC6 0BBE"],
    ["1D5B", "1D5B", "1D5B", "0076", "0076"],
    ["1E2B", "1E2B", "0068 032E", "1E2B", "0068 032E"],
    ["1E8C", "1E8C", "0058 0308", "1E8C", "0058 0308"],
    ["1EF1", "1EF1", "0075 031B 0323", "1EF1", "0075 031B 0323"],
    ["1F64", "1F64", "03C9 0313 0301", "1F64", "03C9 0313 0301"],
    ["1FC9", "0388", "0395 0301", "0388", "0395 0301"],
    ["2087", "2087", "2087", "0037", "0037"],
    ["2163", "2163", "2163", "0049 0056", "0049 0056"],
    ["2471", "2471", "2471", "0031 0038", "0031 0038"],
    ["24D2", "24D2", "24D2", "0063", "0063"],
    ["2F3E", "2F3E", "2F3E", "6236", "6236"],
    ["2F9F", "2F9F", "2F9F", "8F9B", "8F9B"],
    ["30BA", "30BA", "30B9 3099", "30BA", "30B9 3099"],
    ["3179", "3179", "3179", "112C", "112C"],
    ["323E", "323E", "323E", "0028 8CC7 0029", "0028 8CC7 0029"],
    ["32A8", "32A8", "32A8", "53F3", "53F3"],
    ["3309", "3309", "3309", "30AA 30F3 30B9", "30AA 30F3 30B9"],
    ["336A", "336A", "336A", "0031 0038 70B9", "0031 0038 70B9"],
    ["33CB", "33CB", "33CB", "0048 0050", "0048 0050"],
    ["AC1E", "AC1E", "1100 1162 11A9", "AC1E", "1100 1162 11A9"],
    ["AC7F", "AC7F", "1100 1165 11B6", "AC7F", "1100 1165 11B6"],
    ["ACE0", "ACE0", "1100 1169", "ACE0", "1100 1169"],
    ["AD41", "AD41", "1100 116C 11B4", "AD41", "1100 116C 11B4"],
    ["ADA2", "ADA2", "1100 116F 11C1", "ADA2", "1100 116F 11C1"],
    ["AE03", "AE03", "1100 1173 11B2", "AE03", "1100 1173 11B2"],
    ["AE64", "AE64", "1101 1161 11BF", "AE64", "1101 1161 11BF"],
    ["AEC5", "AEC5", "1101 1165 11B0", "AEC5", "1101 1165 11B0"],
    ["AF26", "AF26", "1101 1168 11BD", "AF26", "1101 1168 11BD"],
    ["AF87", "AF87", "1101 116C 11AE", "AF87", "1101 116C 11AE"],
    ["AFE8", "AFE8", "1101 116F 11BB", "AFE8", "1101 116F 11BB"],
    ["B049", "B049", "1101 1173 11AC", "B049", "1101 1173 11AC"],
    ["B0AA", "B0AA", "1102 1161 11B9", "B0AA", "1102 1161 11B9"],
    ["B10B", "B10B", "1102 1165 11AA", "B10B", "1102 1165 11AA"],
    ["B16C", "B16C", "1102 1168 11B7", "B16C", "1102 1168 11B7"],
    ["B1CD", "B1CD", "1102 116C 11A8", "B1CD", "1102 116C 11A8"],
    ["B22E", "B22E", "1102 116F 11B5", "B22E", "1102 116F 11B5"],
    ["B28F", "B28F", "1102 1172 11C2", "B28F", "1102 1172 11C2"],
    ["B2F0", "B2F0", "1103 1161 11B3", "B2F0", "1103 1161 11B3"],
    ["B351", "B351", "1103 1164 11C0", "B351", "1103 1164 11C0"],
    ["B3B2", "B3B2", "1103 1168 11B1", "B3B2", "1103 1168 11B1"],
    ["B413", "B413", "1103 116B 11BE", "B413", "1103 116B 11BE"],
    ["B474", "B474", "1103 116F 11AF", "B474", "1103 116F 11AF"],
    ["B4D5", "B4D5", "1103 1172 11BC", "B4D5", "1103 1172 11BC"],
    ["B536", "B536", "1104 1161 11AD", "B536", "1104 1161 11AD"],
    ["B597", "B597", "1104 1164 11BA", "B597", "1104 1164 11BA"],
    ["B5F8", "B5F8", "1104 1168 11AB", "B5F8", "1104 1168 11AB"],
    ["B659", "B659", "1104 116B 11B8", "B659", "1104 116B 11B8"],
    ["B6BA", "B6BA", "1104 116F 11A9", "B6BA", "1104 116F 11A9"],
    ["B71B", "B71B", "1104 1172 11B6", "B71B", "1104 1172 11B6"],
    ["B77C", "B77C", "1105 1161", "B77C", "1105 1161"],
    ["B7DD", "B7DD", "1105 1164 11B4", "B7DD", "1105 1164 11B4"],
    ["B83E", "B83E", "1105 1167 11C1", "B83E", "1105 1167 11C1"],
    ["B89F", "B89F", "1105 116B 11B2", "B89F", "1105 116B 11B2"],
    ["B900", "B900", "1105 116E 11BF", "B900", "1105 116E 11BF"],
    ["B961", "B961", "1105 1172 11B0", "B961", "1105 1172 11B0"],
    ["B9C2", "B9C2", "1105 1175 11BD", "B9C2", "1105 1175 11BD"],
    ["BA23", "BA23", "1106 1164 11AE", "BA23", "1106 1164 11AE"],
    ["BA84", "BA84", "1106 1167 11BB", "BA84", "1106 1167 11BB"],
    ["BAE5", "BAE5", "1106 116B 11AC", "BAE5", "1106 116B 11AC"],
    ["BB46", "BB46", "1106 116E 11B9", "BB46", "1106 116E 11B9"],
    ["BBA7", "BBA7", "1106 1172 11AA", "BBA7", "1106 1172 11AA"],
    ["BC08", "BC08", "1106 1175 11B7", "BC08", "1106 1175 11B7"],
    ["BC69", "BC69", "1107 1164 11A8", "BC69", "1107 1164 11A8"],
    ["BCCA", "BCCA", "1107 1167 11B5", "BCCA", "1107 1167 11B5"],
    ["BD2B", "BD2B", "1107 116A 11C2", "BD2B", "1107 116A 11C2"],
    ["BD8C", "BD8C", "1107 116E 11B3", "BD8C", "1107 116E 11B3"],
    ["BDED", "BDED", "1107 1171 11C0", "BDED", "1107 1171 11C0"],
    ["BE4E", "BE4E", "1107 1175 11B1", "BE4E", "1107 1175 11B1"],
    ["BEAF", "BEAF", "1108 1163 11BE", "BEAF", "1108 1163 11BE"],
    ["BF10", "BF10", "1108 1167 11AF", "BF10", "1108 1167 11AF"],
    ["BF71", "BF71", "1108 116A 11BC", "BF71", "1108 116A 11BC"],
    ["BFD2", "BFD2", "1108 116E 11AD", "BFD2", "1108 116E 11AD"],
    ["C033", "C033", "1108 1171 11BA", "C033", "1108 1171 11BA"],
    ["C094", "C094", "1108 1175 11AB", "C094", "1108 1175 11AB"],
    ["C0F5", "C0F5", "1109 1163 11B8", "C0F5", "1109 1163 11B8"],
    ["C156", "C156", "1109 1167 11A9", "C156", "1109 1167 11A9"],
    ["C1B7", "C1B7", "1109 116A 11B6", "C1B7", "1109 116A 11B6"],
    ["C218", "C218", "1109 116E", "C218", "1109 116E"],
    ["C279", "C279", "1109 1171 11B4", "C279", "1109 1171 11B4"],
    ["C2DA", "C2DA", "1109 1174 11C1", "C2DA", "1109 1174 11C1"],
    ["C33B", "C33B", "110A 1163 11B2", "C33B", "110A 1163 11B2"],
    ["C39C", "C39C", "110A 1166 11BF", "C39C", "110A 1166 11BF"],
    ["C3FD", "C3FD", "110A 116A 11B0", "C3FD", "110A 116A 11B0"],
    ["C45E", "C45E", "110A 116D 11BD", "C45E", "110A 116D 11BD"],
    ["C4BF", "C4BF", "110A 1171 11AE", "C4BF", "110A 1171 11AE"],
    ["C520", "C520", "110A 1174 11BB", "C520", "110A 1174 11BB"],
    ["C581", "C581", "110B 1163 11AC", "C581", "110B 1163 11AC"],
    ["C5E2", "C5E2", "110B 1166 11B9", "C5E2", "110B 1166 11B9"],
    ["C643", "C643", "110B 116A 11AA", "C643", "110B 116A 11AA"],
    ["C6A4", "C6A4", "110B 116D 11B7", "C6A4", "110B 116D 11B7"],
    ["C705", "C705", "110B 1171 11A8", "C705", "110B 1171 11A8"],
    ["C766", "C766", "110B 1174 11B5", "C766", "110B 1174 11B5"],
    ["C7C7", "C7C7", "110C 1162 11C2", "C7C7", "110C 1162 11C2"],
    ["C828", "C828", "110C 1166 11B3", "C828", "110C 1166 11B3"],
    ["C889", "C889", "110C 1169 11C0", "C889", "110C 1169 11C0"],
    ["C8EA", "C8EA", "110C 116D 11B1", "C8EA", "110C 116D 11B1"],
    ["C94B", "C94B", "110C 1170 11BE", "C94B", "110C 1170 11BE"],
    ["C9AC", "C9AC", "110C 1174 11AF", "C9AC", "110C 1174 11AF"],
    ["CA0D", "CA0D", "110D 1162 11BC", "CA0D", "110D 1162 11BC"],
    ["CA6E", "CA6E", "110D 1166 11AD", "CA6E", "110D 1166 11AD"],
    ["CACF", "CACF", "110D 1169 11BA", "CACF", "110D 1169 11BA"],
    ["CB30", "CB30", "110D 116D 11AB", "CB30", "110D 116D 11AB"],
    ["CB91", "CB91", "110D 1170 11B8", "CB91", "110D 1170 11B8"],
    ["CBF2", "CBF2", "110D 1174 11A9", "CBF2", "110D 1174 11A9"],
    ["CC53", "CC53", "110E 1162 11B6", "CC53", "110E 1162 11B6"],
    ["CCB4", "CCB4", "110E 1166", "CCB4", "110E 1166"],
    ["CD15", "CD15", "110E 1169 11B4", "CD15", "110E 1169 11B4"],
    ["CD76", "CD76", "110E 116C 11C1", "CD76", "110E 116C 11C1"],
    ["CDD7", "CDD7", "110E 1170 11B2", "CDD7", "110E 1170 11B2"],
    ["CE38", "CE38", "110E 1173 11BF", "CE38", "110E 1173 11BF"],
    ["CE99", "CE99", "110F 1162 11B0", "CE99", "110F 1162 11B0"],
    ["CEFA", "CEFA", "110F 1165 11BD", "CEFA", "110F 1165 11BD"],
    ["CF5B", "CF5B", "110F 1169 11AE", "CF5B", "110F 1169 11AE"],
    ["CFBC", "CFBC", "110F 116C 11BB", "CFBC", "110F 116C 11BB"],
    ["D01D", "D01D", "110F 1170 11AC", "D01D", "110F 1170 11AC"],
    ["D07E", "D07E", "110F 1173 11B9", "D07E", "110F 1173 11B9"],
    ["D0DF", "D0DF", "1110 1162 11AA", "D0DF", "1110 1162 11AA"],
    ["D140", "D140", "1110 1165 11B7", "D140", "1110 1165 11B7"],
    ["D1A1", "D1A1", "1110 1169 11A8", "D1A1", "1110 1169 11A8"],
    ["D202", "D202", "1110 116C 11B5", "D202", "1110 116C 11B5"],
    ["D263", "D263", "1110 116F 11C2", "D263", "1110 116F 11C2"],
    ["D2C4", "D2C4", "1110 1173 11B3", "D2C4", "1110 1173 11B3"],
    ["D325", "D325", "1111 1161 11C0", "D325", "1111 1161 11C0"],
    ["D386", "D386", "1111 1165 11B1", "D386", "1111 1165 11B1"],
    ["D3E7", "D3E7", "1111 1168 11BE", "D3E7", "1111 1168 11BE"],
    ["D448", "D448", "1111 116C 11AF", "D448", "1111 116C 11AF"],
    ["D4A9", "D4A9", "1111 116F 11BC", "D4A9", "1111 116F 11BC"],
    ["D50A", "D50A", "1111 1173 11AD", "D50A", "1111 1173 11AD"],
    ["D56B", "D56B", "1112 1161 11BA", "D56B", "1112 1161 11BA"],
    ["D5CC", "D5CC", "1112 1165 11AB", "D5CC", "1112 1165 11AB"],
    ["D62D", "D62D", "1112 1168 11B8", "D62D", "1112 1168 11B8"],
    ["D68E", "D68E", "1112 116C 11A9", "D68E", "1112 116C 11A9"],
    ["D6EF", "D6EF", "1112 116F 11B6", "D6EF", "1112 116F 11B6"],
    ["D750", "D750", "1112 1173", "D750", "1112 1173"],
    ["F90D", "61F6", "61F6", "61F6", "61F6"],
    ["F96E", "8449", "8449", "8449", "8449"],
    ["F9CF", "7D10", "7D10", "7D10", "7D10"],
    ["FA3C", "5C6E", "5C6E", "5C6E", "5C6E"],
    ["FA9F", "72AF", "72AF", "72AF", "72AF"],
    ["FB39", "05D9 05BC", "05D9 05BC", "05D9 05BC", "05D9 05BC"],
    ["FB9E", "FB9E", "FB9E", "06BA", "06BA"],
    ["FC20", "FC20", "FC20", "0635 062D", "0635 062D"],
    ["FC81", "FC81", "FC81", "0643 0644", "0643 0644"],
    ["FCE2", "FCE2", "FCE2", "0628 0647", "0628 0647"],
    ["FD55", "FD55", "FD55", "062A 0645 062C", "062A 0645 062C"],
    ["FDB8", "FDB8", "FDB8", "0646 062C 062D", "0646 062C 062D"],
    ["FE72", "FE72", "FE72", "0020 064C", "0020 064C"],
    ["FED5", "FED5", "FED5", "0642", "0642"],
    ["FF3A", "FF3A", "FF3A", "005A", "005A"],
    ["FF9B", "FF9B", "FF9B", "30ED", "30ED"],
    ["1079A", "1079A", "1079A", "02AB", "02AB"],
    ["1CCF8", "1CCF8", "1CCF8", "0038", "0038"],
    ["1D452", "1D452", "1D452", "0065", "0065"],
    ["1D4BE", "1D4BE", "1D4BE", "0069", "0069"],
    ["1D525", "1D525", "1D525", "0068", "0068"],
    ["1D58D", "1D58D", "1D58D", "0068", "0068"],
    ["1D5EE", "1D5EE", "1D5EE", "0061", "0061"],
    ["1D64F", "1D64F", "1D64F", "0054", "0054"],
    ["1D6B2", "1D6B2", "1D6B2", "039B", "039B"],
    ["1D713", "1D713", "1D713", "03C8", "03C8"],
    ["1D774", "1D774", "1D774", "03B5", "03B5"],
    ["1D7D7", "1D7D7", "1D7D7", "0039", "0039"],
    ["1E068", "1E068", "1E068", "0456", "0456"],
    ["1EE82", "1EE82", "1EE82", "062C", "062C"],
    ["1F135", "1F135", "1F135", "0046", "0046"],
    ["1FBF8", "1FBF8", "1FBF8", "0038", "0038"],
    ["2F85F", "5962", "5962", "5962", "5962"],
    ["2F8C0", "63C5", "63C5", "63C5", "63C5"],
    ["2F921", "7235", "7235", "7235", "7235"],
    ["2F982", "80B2", "80B2", "80B2", "80B2"],
    ["2F9E3", "90F1", "90F1", "90F1", "90F1"],
    [
        "0061 0315 0300 05AE 0313 0062",
        "00E0 05AE 0313 0315 0062",
        "0061 05AE 0300 0313 0315 0062",
        "00E0 05AE 0313 0315 0062",
        "0061 05AE 0300 0313 0315 0062",
    ],
    [
        "0061 0343 0315 0300 05AE 0062",
        "0061 05AE 0313 0300 0315 0062",
        "0061 05AE 0313 0300 0315 0062",
        "0061 05AE 0313 0300 0315 0062",
        "0061 05AE 0313 0300 0315 0062",
    ],
    [
        "0061 059A 0316 1DFA 0591 0062",
        "0061 1DFA 0316 0591 059A 0062",
        "0061 1DFA 0316 0591 059A 0062",
        "0061 1DFA 0316 0591 059A 0062",
        "0061 1DFA 0316 0591 059A 0062",
    ],
    [
        "0061 05C4 0315 0300 05AE 0062",
        "0061 05AE 05C4 0300 0315 0062",
        "0061 05AE 05C4 0300 0315 0062",
        "0061 05AE 05C4 0300 0315 0062",
        "0061 05AE 05C4 0300 0315 0062",
    ],
    [
        "0061 0315 0300 05AE 06E7 0062",
        "00E0 05AE 06E7 0315 0062",
        "0061 05AE 0300 06E7 0315 0062",
        "00E0 05AE 06E7 0315 0062",
        "0061 05AE 0300 06E7 0315 0062",
    ],
    [
        "0061 081B 0315 0300 05AE 0062",
        "0061 05AE 081B 0300 0315 0062",
        "0061 05AE 081B 0300 0315 0062",
        "0061 05AE 081B 0300 0315 0062",
        "0061 05AE 081B 0300 0315 0062",
    ],
    [
        "0061 0315 0300 05AE 08DE 0062",
        "00E0 05AE 08DE 0315 0062",
        "0061 05AE 0300 08DE 0315 0062",
        "00E0 05AE 08DE 0315 0062",
        "0061 05AE 0300 08DE 0315 0062",
    ],
    [
        "0061 0BCD 05B0 094D 3099 0062",
        "0061 3099 0BCD 094D 05B0 0062",
        "0061 3099 0BCD 094D 05B0 0062",
        "0061 3099 0BCD 094D 05B0 0062",
        "0061 3099 0BCD 094D 05B0 0062",
    ],
    [
        "0061 0315 0300 05AE 135E 0062",
        "00E0 05AE 135E 0315 0062",
        "0061 05AE 0300 135E 0315 0062",
        "00E0 05AE 135E 0315 0062",
        "0061 05AE 0300 135E 0315 0062",
    ],
    [
        "0061 1ACA 059A 0316 1DFA 0062",
        "0061 1DFA 1ACA 0316 059A 0062",
        "0061 1DFA 1ACA 0316 059A 0062",
        "0061 1DFA 1ACA 0316 059A 0062",
        "0061 1DFA 1ACA 0316 059A 0062",
    ],
    [
        "0061 0315 0300 05AE 1CD0 0062",
        "00E0 05AE 1CD0 0315 0062",
        "0061 05AE 0300 1CD0 0315 0062",
        "00E0 05AE 1CD0 0315 0062",
        "0061 05AE 0300 1CD0 0315 0062",
    ],
    [
        "0061 1DD5 0315 0300 05AE 0062",
        "0061 05AE 1DD5 0300 0315 0062",
        "0061 05AE 1DD5 0300 0315 0062",
        "0061 05AE 1DD5 0300 0315 0062",
        "0061 05AE 1DD5 0300 0315 0062",
    ],
    [
        "0061 0315 0300 05AE 20D6 0062",
        "00E0 05AE 20D6 0315 0062",
        "0061 05AE 0300 20D6 0315 0062",
        "00E0 05AE 20D6 0315 0062",
        "0061 05AE 0300 20D6 0315 0062",
    ],
    [
        "0061 2DF8 0315 0300 05AE 0062",
        "0061 05AE 2DF8 0300 0315 0062",
        "0061 05AE 2DF8 0300 0315 0062",
        "0061 05AE 2DF8 0300 0315 0062",
        "0061 05AE 2DF8 0300 0315 0062",
    ],
    [
        "0061 0315 0300 05AE A8EF 0062",
        "00E0 05AE A8EF 0315 0062",
        "0061 05AE 0300 A8EF 0315 0062",
        "00E0 05AE A8EF 0315 0062",
        "0061 05AE 0300 A8EF 0315 0062",
    ],
    [
        "0061 10A3A 059A 0316 1DFA 0062",
        "0061 1DFA 10A3A 0316 059A 0062",
        "0061 1DFA 10A3A 0316 059A 0062",
        "0061 1DFA 10A3A 0316 059A 0062",
        "0061 1DFA 10A3A 0316 059A 0062",
    ],
    [
        "0061 3099 093C 16FF0 11236 0062",
        "0061 16FF0 093C 11236 3099 0062",
        "0061 16FF0 093C 11236 3099 0062",
        "0061 16FF0 093C 11236 3099 0062",
        "0061 16FF0 093C 11236 3099 0062",
    ],
    [
        "0061 1612F 05B0 094D 3099 0062",
        "0061 3099 1612F 094D 05B0 0062",
        "0061 3099 1612F 094D 05B0 0062",
        "0061 3099 1612F 094D 05B0 0062",
        "0061 3099 1612F 094D 05B0 0062",
    ],
    [
        "0061 0315 0300 05AE 1E000 0062",
        "00E0 05AE 1E000 0315 0062",
        "0061 05AE 0300 1E000 0315 0062",
        "00E0 05AE 1E000 0315 0062",
        "0061 05AE 0300 1E000 0315 0062",
    ],
    [
        "0061 1E2ED 0315 0300 05AE 0062",
        "0061 05AE 1E2ED 0300 0315 0062",
        "0061 05AE 1E2ED 0300 0315 0062",
        "0061 05AE 1E2ED 0300 0315 0062",
        "0061 05AE 1E2ED 0300 0315 0062",
    ],
    [
        "1106 0334 1165",
        "1106 0334 1165",
        "1106 0334 1165",
        "1106 0334 1165",
        "1106 0334 1165",
    ],
    [
        "CAD8 0334 11BC",
        "CAD8 0334 11BC",
        "110D 116A 0334 11BC",
        "CAD8 0334 11BC",
        "110D 116A 0334 11BC",
    ],
    [
        "0DDC 0DCA",
        "0DDD",
        "0DD9 0DCF 0DCA",
        "0DDD",
        "0DD9 0DCF 0DCA",
    ],
    [
        "00CA 0309",
        "1EC2",
        "0045 0302 0309",
        "1EC2",
        "0045 0302 0309",
    ],
    [
        "1F10 0341",
        "1F14",
        "03B5 0313 0301",
        "1F14",
        "03B5 0313 0301",
    ],
    [
        "1F68 0341",
        "1F6C",
        "03A9 0313 0301",
        "1F6C",
        "03A9 0313 0301",
    ],
    [
        "1F0D 0345",
        "1F8D",
        "0391 0314 0301 0345",
        "1F8D",
        "0391 0314 0301 0345",
    ],
    [
        "1F29 0345 0300",
        "1F9B",
        "0397 0314 0300 0345",
        "1F9B",
        "0397 0314 0300 0345",
    ],
    [
        "1FA8 0300",
        "1FAA",
        "03A9 0313 0300 0345",
        "1FAA",
        "03A9 0313 0300 0345",
    ],
    [
        "16122 1611F",
        "16127",
        "1611E 16129 1611F",
        "16127",
        "1611E 16129 1611F",
    ],
];

#[test]
fn normalize_keeps_the_conformance_files_invariants() {
    let mut src = String::from("var rows = [");
    for r in CONFORMANCE {
        src.push_str(&format!(
            "[\"{}\",\"{}\",\"{}\",\"{}\",\"{}\"],",
            r[0], r[1], r[2], r[3], r[4]
        ));
    }
    src.push_str(
        "]; function str(h) { return String.fromCodePoint.apply(null, h.split(' ').map(function (x) { return parseInt(x, 16); })); } \
         var bad = []; \
         function check(l, g, w, r) { if (g !== w) bad.push(l + ' ' + r.join(';')); } \
         rows.forEach(function (r) { var c = r.map(str); \
           for (var i = 0; i < 3; i++) { check('NFC', c[i].normalize('NFC'), c[1], r); check('NFD', c[i].normalize('NFD'), c[2], r); } \
           for (var j = 3; j < 5; j++) { check('NFC', c[j].normalize('NFC'), c[3], r); check('NFD', c[j].normalize('NFD'), c[4], r); } \
           for (var k = 0; k < 5; k++) { check('NFKC', c[k].normalize('NFKC'), c[3], r); check('NFKD', c[k].normalize('NFKD'), c[4], r); } }); \
         rows.length + ' ' + bad.join(' | ')",
    );
    assert_eq!(run(&src), format!("{} ", CONFORMANCE.len()));
}
