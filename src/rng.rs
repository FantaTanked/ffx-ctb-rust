const RNG_CONSTANTS_1: [i64; 68] = [
    2100005341, 1700015771, 247163863, 891644838, 1352476256, 1563244181,
    1528068162, 511705468, 1739927914, 398147329, 1278224951, 20980264,
    1178761637, 802909981, 1130639188, 1599606659, 952700148, -898770777,
    -1097979074, -2013480859, -338768120, -625456464, -2049746478, -550389733,
    -5384772, -128808769, -1756029551, 1379661854, 904938180, -1209494558,
    -1676357703, -1287910319, 1653802906, 393811311, -824919740, 1837641861,
    946029195, 1248183957, -1684075875, -2108396259, -681826312, 1003979812,
    1607786269, -585334321, 1285195346, 1997056081, -106688232, 1881479866,
    476193932, 307456100, 1290745818, 162507240, -213809065, -1135977230,
    -1272305475, 1484222417, -1559875058, 1407627502, 1206176750, -1537348094,
    638891383, 581678511, 1164589165, -1436620514, 1412081670, -1538191350,
    -284976976, 706005400,
];

const RNG_CONSTANTS_2: [i64; 68] = [
    10259, 24563, 11177, 56952, 46197, 49826, 27077, 1257, 44164, 56565, 31009,
    46618, 64397, 46089, 58119, 13090, 19496, 47700, 21163, 16247, 574, 18658,
    60495, 42058, 40532, 13649, 8049, 25369, 9373, 48949, 23157, 32735, 29605,
    44013, 16623, 15090, 43767, 51346, 28485, 39192, 40085, 32893, 41400, 1267,
    15436, 33645, 37189, 58137, 16264, 59665, 53663, 11528, 37584, 18427,
    59827, 49457, 22922, 24212, 62787, 56241, 55318, 9625, 57622, 7580, 56469,
    49208, 41671, 36458,
];

#[derive(Debug, Clone)]
pub struct FfxRngTracker {
    pub seed: u32,
    initial_values: [u32; 68],
    current_values: [i64; 68],
    current_positions: [usize; 68],
    arrays: Vec<Vec<u32>>,
}

impl FfxRngTracker {
    pub fn new(seed: u32) -> Self {
        let initial_values = initial_values(seed);
        let mut current_values = [0; 68];
        for (index, value) in initial_values.iter().enumerate() {
            current_values[index] = to_i32(*value as i64);
        }
        Self {
            seed,
            initial_values,
            current_values,
            current_positions: [0; 68],
            arrays: vec![Vec::new(); 68],
        }
    }

    pub fn initial_values(&self) -> [u32; 68] {
        self.initial_values
    }

    pub fn current_positions(&self) -> [usize; 68] {
        self.current_positions
    }

    pub fn advance_rng(&mut self, index: usize) -> u32 {
        assert!(index < 68, "rng index must be between 0 and 67");
        let position = self.current_positions[index];
        self.current_positions[index] = position + 1;
        if let Some(value) = self.arrays[index].get(position) {
            return *value;
        }
        let (next_state, value) = next_value(
            self.current_values[index],
            RNG_CONSTANTS_1[index],
            RNG_CONSTANTS_2[index],
        );
        self.current_values[index] = next_state;
        self.arrays[index].push(value);
        value
    }

    pub fn reset(&mut self) {
        self.current_positions = [0; 68];
    }
}

fn initial_values(seed: u32) -> [u32; 68] {
    let mut rng_value = to_i32(seed as i64);
    let mut values = [0; 68];
    for value in &mut values {
        rng_value = to_i32(rng_value * 0x5d588b65 + 0x3c35);
        rng_value = rotate_halves_like_python(rng_value);
        *value = (rng_value & 0x7fff_ffff) as u32;
    }
    values
}

fn next_value(current: i64, constant_1: i64, constant_2: i64) -> (i64, u32) {
    let multiplied = current * constant_1;
    let xored = to_i32((multiplied as i32 ^ constant_2 as i32) as i64);
    let rotated = rotate_halves_like_python(xored);
    (rotated, (rotated & 0x7fff_ffff) as u32)
}

fn rotate_halves_like_python(value: i64) -> i64 {
    to_i32((value >> 16) + (value << 16))
}

fn to_i32(value: i64) -> i64 {
    value as i32 as i64
}

#[cfg(test)]
mod tests {
    use super::FfxRngTracker;

    #[test]
    fn matches_python_initial_values_for_default_seed() {
        let tracker = FfxRngTracker::new(3096296922);
        assert_eq!(
            &tracker.initial_values()[..8],
            &[20396785, 1481230243, 8633225, 474117860, 1881763622, 942883522, 1757343272, 1778181219]
        );
    }

    #[test]
    fn matches_python_rolls_for_default_seed() {
        let mut tracker = FfxRngTracker::new(3096296922);
        assert_eq!(
            (0..5).map(|_| tracker.advance_rng(0)).collect::<Vec<_>>(),
            vec![1931357875, 865355549, 1092221934, 1919230522, 1040282205]
        );
        assert_eq!(
            (0..5).map(|_| tracker.advance_rng(1)).collect::<Vec<_>>(),
            vec![1984005895, 1557033276, 145169671, 181243313, 1205357822]
        );
        assert_eq!(
            (0..5).map(|_| tracker.advance_rng(4)).collect::<Vec<_>>(),
            vec![1714765911, 1054138592, 544543381, 563380746, 196379424]
        );
        assert_eq!(
            (0..5).map(|_| tracker.advance_rng(20)).collect::<Vec<_>>(),
            vec![688806317, 844487467, 476435584, 641579266, 1680724002]
        );
    }

    #[test]
    fn matches_python_rolls_for_small_seeds() {
        let mut tracker = FfxRngTracker::new(1);
        assert_eq!(
            &tracker.initial_values()[..8],
            &[1201298776, 1475124949, 222143696, 843409743, 761293752, 600613063, 934824559, 1593865543]
        );
        assert_eq!(
            (0..5).map(|_| tracker.advance_rng(20)).collect::<Vec<_>>(),
            vec![63858152, 1165884297, 561395089, 1773524655, 876985368]
        );

        let mut tracker = FfxRngTracker::new(0);
        assert_eq!(
            &tracker.initial_values()[..8],
            &[1010106368, 1010075625, 1579262877, 136687124, 1176028994, 792663207, 253241474, 1300183350]
        );
        assert_eq!(
            (0..5).map(|_| tracker.advance_rng(4)).collect::<Vec<_>>(),
            vec![817154648, 1165275596, 1089808450, 1354043490, 78958702]
        );
    }
}
