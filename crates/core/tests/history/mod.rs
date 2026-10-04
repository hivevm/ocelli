//! A deterministic generator of synthetic commit histories for the layout's reference test.
//! The same generator produced `fixtures/layout.sha256` from GitGraph's `build_graph` at commit
//! 9dd6ec9 of `main`: for each seed, the SHA-256 of `serde_json::to_string` of the graph.

pub struct Xorshift(u64);

impl Xorshift {
    pub fn new(seed: u64) -> Self {
        Xorshift(seed.max(1))
    }
    pub fn next(&mut self, n: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as usize
    }
}

pub struct Spec {
    pub sha: String,
    pub message: String,
    pub author_name: String,
    pub author_email: String,
    pub author_time: i64,
    pub committer_name: String,
    pub committer_email: String,
    pub commit_time: i64,
    pub refs: Vec<String>,
    pub parent_shas: Vec<String>,
    pub kind: u8, // 0 commit, 1 stash, 2 working
}

pub fn history(seed: u64) -> Vec<Spec> {
    let mut r = Xorshift::new(seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ 0x5851_f42d);
    let len = 1 + r.next(160);
    let roots = 1 + r.next(3);
    let names = ["Ada", "Linus", "Grace", "Ken", ""];
    let mut specs: Vec<Spec> = Vec::new();
    let mut tips: Vec<String> = Vec::new();
    let mut time: i64 = 1_700_000_000;
    for i in 0..len {
        // Ties in committer time, one commit in five.
        if r.next(5) != 0 {
            time += 1 + r.next(5000) as i64;
        }
        let sha = format!("{:040x}", (seed << 20) ^ (i as u64 + 1));
        let mut parents: Vec<String> = Vec::new();
        if i >= roots && !tips.is_empty() {
            let t = r.next(tips.len());
            parents.push(tips[t].clone());
            if tips.len() > 1 && r.next(4) == 0 {
                let o = (t + 1 + r.next(tips.len() - 1)) % tips.len();
                parents.push(tips[o].clone());
                // Octopus merge, rarely.
                if tips.len() > 2 && r.next(6) == 0 {
                    let p = (o + 1) % tips.len();
                    if p != t {
                        parents.push(tips[p].clone());
                    }
                }
            }
            // A parent outside the loaded set, rarely.
            if r.next(25) == 0 {
                parents.push(format!("{:040x}", 0xdead_0000_u64 + i as u64));
            }
        }
        for merged in parents.iter().skip(1) {
            tips.retain(|x| x != merged);
        }
        if let Some(first) = parents.first() {
            if let Some(t) = tips.iter().position(|x| x == first) {
                if r.next(3) == 0 {
                    tips.push(sha.clone());
                } else {
                    tips[t] = sha.clone();
                }
            } else {
                tips.push(sha.clone());
            }
        } else {
            tips.push(sha.clone());
        }
        let author = names[r.next(names.len())];
        let committer = names[r.next(names.len())];
        let mut refs = Vec::new();
        if r.next(7) == 0 {
            refs.push(format!("feature/{i}"));
        }
        if r.next(13) == 0 {
            refs.push(format!("\u{1f3f7} v{i}"));
        }
        specs.push(Spec {
            sha,
            message: format!("commit {i}"),
            author_name: author.to_string(),
            author_email: if author.is_empty() {
                String::new()
            } else {
                format!(" {}@Example.org ", author)
            },
            author_time: time - r.next(100_000) as i64,
            committer_name: committer.to_string(),
            committer_email: if committer.is_empty() {
                String::new()
            } else {
                format!("{}@example.org", committer.to_lowercase())
            },
            commit_time: time,
            refs,
            parent_shas: parents,
            kind: 0,
        });
    }
    // Synthetic nodes: a stash on some commit, and a working-directory node on top.
    if len > 3 && r.next(3) == 0 {
        let base = specs[r.next(specs.len())].sha.clone();
        time += 10;
        specs.push(Spec {
            sha: format!("{:040x}", (seed << 20) ^ 0xfffff),
            message: "WIP on main".into(),
            author_name: "Ada".into(),
            author_email: "ada@example.org".into(),
            author_time: time,
            committer_name: "Ada".into(),
            committer_email: "ada@example.org".into(),
            commit_time: time,
            refs: vec!["\u{1f4e6} stash@{0}".into()],
            parent_shas: vec![base],
            kind: 1,
        });
    }
    if r.next(2) == 0 {
        let head = tips[r.next(tips.len())].clone();
        specs.push(Spec {
            sha: "WORKING_DIRECTORY".into(),
            message: "Uncommitted changes".into(),
            author_name: String::new(),
            author_email: String::new(),
            author_time: time + 100,
            committer_name: String::new(),
            committer_email: String::new(),
            commit_time: time + 100,
            refs: Vec::new(),
            parent_shas: vec![head],
            kind: 2,
        });
    }
    specs
}
