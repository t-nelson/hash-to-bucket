use {
    serde::{de, Deserialize, Deserializer},
    serde_json::Value as JsonValue,
    solana_sdk::pubkey::Pubkey,
    std::{
        collections::HashMap,
        hash::{BuildHasher, Hasher},
        ops::Deref,
    },
};

const BUCKETS: usize = 100;
const EPOCHS: u64 = 1000;

#[derive(Clone)]
struct Blake3Hasher(blake3::Hasher);

impl Hasher for Blake3Hasher {
    fn finish(&self) -> u64 {
        let hash = self.0.finalize();
        u64::from_le_bytes(hash.as_bytes()[..8].try_into().unwrap())
    }

    fn write(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }
}

impl Blake3Hasher {
    fn new_with_seed(seed: u64) -> Self {
        let seed_bytes = seed.to_le_bytes();
        let mut key = [0u8; 32];
        for chunk in key.chunks_mut(8) {
            chunk.copy_from_slice(&seed_bytes);
        }
        Self(blake3::Hasher::new_keyed(&key))
    }
}

fn de_stringified_pubkey<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Pubkey, D::Error> {
    match JsonValue::deserialize(deserializer)? {
        JsonValue::String(s) => s.parse().map_err(de::Error::custom),
        _ => Err(de::Error::custom("wrong type")),
    }
}

#[derive(Debug, Deserialize)]
struct Pubkey2(
    #[serde(deserialize_with = "de_stringified_pubkey")]
    Pubkey
);

impl Deref for Pubkey2 {
    type Target = Pubkey;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug)]
struct BucketAnalysis {
    pub min: usize,
    pub max: usize,
    pub spread: usize,
    pub mean: usize,
    pub median: usize,
    pub mode: usize,
    pub mode_count: usize,
    pub std_dev: f64,
}

impl std::fmt::Display for BucketAnalysis {
    fn fmt(&self, formatter: &mut std::fmt::Formatter) -> Result<(), std::fmt::Error> {
        write!(formatter, "{},{},{},{},{},{},{},{}", self.min, self.max, self.spread, self.mean, self.median, self.mode, self.mode_count, self.std_dev)
    }
}

fn analyze_buckets(buckets: &mut [usize]) -> BucketAnalysis {
    buckets.sort();
    let min = buckets[0];
    let max = buckets[BUCKETS - 1];
    let spread = max - min;
    let sum = buckets.iter().sum::<usize>();
    let mean = sum / BUCKETS;
    let median = buckets[BUCKETS / 2];
    let mut freq = HashMap::new();
    for bucket in buckets.iter() {
        freq.entry(*bucket)
            .and_modify(|f| *f += 1)
            .or_insert(1);
    }
    let mut freq = freq.iter().collect::<Vec<_>>();
    freq.sort_by_key(|(_,v)| *v);
    let (mode, mode_count) = freq.last().map(|(k, v)| (**k, **v)).unwrap();
    let std_dev = buckets.iter().map(|count| (*count as f64 - mean as f64).abs()).sum::<f64>() / (BUCKETS as f64);

    BucketAnalysis { min, max, spread, mean, median, mode, mode_count, std_dev }
}

#[allow(dead_code)]
fn address_to_bucket(buckets: usize, epoch: u64, address: &Pubkey2) -> usize {
    let state = ahash::random_state::RandomState::with_seeds(epoch, epoch, epoch, epoch);
    let hasher = state.build_hasher();
    address_to_bucket_with_epoch_hasher(buckets, hasher, address)
}

fn address_to_bucket_with_epoch_hasher<H: Hasher>(buckets: usize, mut hasher: H, address: &Pubkey2) -> usize {
    hasher.write(address.as_ref());
    let hash = hasher.finish();
    ((buckets as u128) * (hash as u128) / ((u64::MAX as u128) + 1)) as usize
}

fn do_test<H: Hasher + Clone>(hasher: H, epoch: u64, addresses: &[Pubkey2]) -> std::time::Duration {
    let mut buckets = Vec::with_capacity(BUCKETS);
    buckets.resize(BUCKETS, 0);
    let start = std::time::Instant::now();
    for address in addresses {
        let bucket = address_to_bucket_with_epoch_hasher(BUCKETS, hasher.clone(), address);
        buckets[bucket] += 1;
    }
    let time = std::time::Instant::now().duration_since(start);
    println!("{epoch},{}", analyze_buckets(&mut buckets));
    time
}

fn main() {
    let file = std::fs::File::open("./addresses.json").unwrap();
    let reader = std::io::BufReader::new(file);
    let addresses: Vec<Pubkey2> = serde_json::from_reader(reader).unwrap();
    let mut timings = HashMap::new();
    println!("epoch,min,max,spread,mean,median,mode,mode_count,std_dev");
    for epoch in 0u64..EPOCHS {
        for (name, time) in [
            /*
            ("ahash", {
                let state = ahash::random_state::RandomState::with_seeds(epoch, epoch, epoch, epoch);
                let hasher = state.build_hasher();
                do_test(hasher, epoch, &addresses)
            }),
            ("siphash24", {
                let hasher = siphasher::sip::SipHasher24::new_with_keys(epoch, epoch);
                do_test(hasher, epoch, &addresses)
            }),
            ("siphash13", {
                let hasher = siphasher::sip::SipHasher13::new_with_keys(epoch, epoch);
                do_test(hasher, epoch, &addresses)
            }),
            ("murmur3", {
                let hasher = mur3::Hasher128::with_seed(epoch as u32);
                do_test(hasher, epoch, &addresses)
            }),
            */
            ("blake3", {
                let hasher = Blake3Hasher::new_with_seed(epoch);
                do_test(hasher, epoch, &addresses)
            }),
        ].iter() {
            timings.entry(name.to_string())
                .and_modify(|v: &mut std::time::Duration| *v += *time)
                .or_insert(*time);
        }
    }

    for (name, time) in timings.into_iter() {
        println!("{name}: {}",  (time / (EPOCHS as u32)).as_micros());
    }
}
