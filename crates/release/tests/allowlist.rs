//! SITE-025 task 5.17, security review L2, L3, L4 and L6 (workspace 5.21i): the attributes that the
//! HTML allowlist keeps, case by case, and the property that sanitising twice changes nothing.

use ratatoskr_sites_release::sanitise::sanitise;

const ASSET: &str = "assets/ced0e0da2723c17bce89eac3ffc3107ca7753d95c6abc28ce29214a19667728d";

#[test]
fn attributes_follow_the_allowlist() {
    let long_lang = format!("<code class=\"language-{}\">x</code>", "a".repeat(40));
    let asset_image = format!("<img src=\"{ASSET}\" alt=\"a\">");
    let cases: Vec<(&str, String, &str)> = vec![
        // L2: class values only from the set of the renderer.
        (
            "a callout class stays",
            "<div class=\"callout\">x</div>".into(),
            "<div class=\"callout\">x</div>",
        ),
        (
            "an unknown class goes",
            "<div class=\"callout evil\">x</div>".into(),
            "<div class=\"callout\">x</div>",
        ),
        (
            "a div without a known class has none",
            "<div class=\"evil\">x</div>".into(),
            "<div>x</div>",
        ),
        (
            "a language class stays",
            "<code class=\"language-rust\">x</code>".into(),
            "<code class=\"language-rust\">x</code>",
        ),
        (
            "a second code class goes",
            "<code class=\"language-rust extra\">x</code>".into(),
            "<code class=\"language-rust\">x</code>",
        ),
        ("a long language goes", long_lang, "<code>x</code>"),
        (
            "a class on a link goes",
            "<a class=\"callout\" href=\"https://example.test\">x</a>".into(),
            "<a href=\"https://example.test\">x</a>",
        ),
        // L3: heading ids only of lower-case ASCII letters, digits and `-`.
        (
            "a heading id stays",
            "<h2 id=\"start-here\">x</h2>".into(),
            "<h2 id=\"start-here\">x</h2>",
        ),
        (
            "a heading id with a space goes",
            "<h2 id=\"Start Here\">x</h2>".into(),
            "<h2>x</h2>",
        ),
        (
            "a heading id with markup goes",
            "<h3 id=\"a&quot;b\">x</h3>".into(),
            "<h3>x</h3>",
        ),
        // L4: an image only from the assets of the release or over https.
        (
            "an asset image stays",
            asset_image.clone(),
            asset_image.as_str(),
        ),
        (
            "an https image stays",
            "<img src=\"https://example.test/a.png\">".into(),
            "<img src=\"https://example.test/a.png\">",
        ),
        (
            "an http image loses its source",
            "<img src=\"http://example.test/a.png\">".into(),
            "<img>",
        ),
        (
            "a protocol-relative image loses its source",
            "<img src=\"//evil.test/a.png\">".into(),
            "<img>",
        ),
        (
            "a same-origin image loses its source",
            "<img src=\"/v1/sites/s/content/release\">".into(),
            "<img>",
        ),
        (
            "an asset path that climbs loses its source",
            "<img src=\"assets/../secret\">".into(),
            "<img>",
        ),
        (
            "an asset without a digest loses its source",
            "<img src=\"assets/ABC\">".into(),
            "<img>",
        ),
        (
            "a javascript link loses its target",
            "<a href=\"javascript:alert(1)\">x</a>".into(),
            "<a>x</a>",
        ),
    ];
    for (case, input, expected) in &cases {
        assert_eq!(sanitise(input), *expected, "{case}");
    }
}

/// A fixed sequence of numbers, so the property test is the same on every run.
struct Sequence(u64);

impl Sequence {
    fn next(&mut self, below: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        usize::try_from(self.0 % u64::try_from(below).unwrap_or(1)).unwrap_or(0)
    }
}

const FRAGMENTS: [&str; 30] = [
    "<b>",
    "</b>",
    "<div/>",
    "<p>",
    "</p>",
    "<a href=\"javascript:x\">",
    "</a>",
    "<script>",
    "</script>",
    "<style/>",
    "x",
    "&amp;",
    "<img src=\"//e\">",
    "<ul><li>",
    "</li>",
    "<table>",
    "<tr><td>",
    "<h1 id=\"A b\">",
    "</h1>",
    "<!--c-->",
    "<svg><a>",
    "</svg>",
    "\"",
    "<code class=\"language-rs evil\">",
    "</code>",
    "<embed>",
    "<input type=text>",
    "<input type=checkbox checked>",
    "<div class=\"callout\" data-callout-fold=\"open\">",
    "</div>",
];

#[test]
fn sanitising_twice_changes_nothing() {
    let mut sequence = Sequence(0x2545_f491_4f6c_dd1d);
    for _ in 0..500 {
        let length = 1 + sequence.next(30);
        let input: String = (0..length)
            .map(|_| FRAGMENTS[sequence.next(FRAGMENTS.len())])
            .collect();
        let once = sanitise(&input);
        assert_eq!(
            sanitise(&once),
            once,
            "sanitising {input:?} twice changes nothing"
        );
    }
}
