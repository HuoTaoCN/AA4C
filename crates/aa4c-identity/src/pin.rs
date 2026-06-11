//! 配对 PIN 推导（PROTOCOL.md §6.1）。
//!
//! PIN 由两端各自独立计算、独立显示，不经网络传输；
//! 与公钥顺序无关（按字典序归一化），中间人无法令两端 PIN 一致。

/// 由双方公钥推导 6 位确认码：
/// `pin = u32::from_le_bytes(BLAKE3(min(pkA,pkB) ‖ max(pkA,pkB))[0..4]) % 1_000_000`
pub fn derive_pin(pk_a: &[u8], pk_b: &[u8]) -> String {
    let (lo, hi) = if pk_a <= pk_b {
        (pk_a, pk_b)
    } else {
        (pk_b, pk_a)
    };
    let mut hasher = blake3::Hasher::new();
    hasher.update(lo);
    hasher.update(hi);
    let digest = hasher.finalize();
    let n = u32::from_le_bytes(
        digest.as_bytes()[0..4]
            .try_into()
            .expect("blake3 digest is at least 4 bytes"),
    );
    format!("{:06}", n % 1_000_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_is_order_independent_and_six_digits() {
        let pk_a = [1u8; 32];
        let pk_b = [2u8; 32];
        let pin_ab = derive_pin(&pk_a, &pk_b);
        let pin_ba = derive_pin(&pk_b, &pk_a);
        assert_eq!(pin_ab, pin_ba);
        assert_eq!(pin_ab.len(), 6);
        assert!(pin_ab.chars().all(|c| c.is_ascii_digit()));
    }

    #[test]
    fn different_keys_yield_different_pins() {
        let pk_a = [1u8; 32];
        let pk_b = [2u8; 32];
        let pk_c = [3u8; 32];
        // 攻击者替换一侧公钥后 PIN 必然变化（碰撞概率 1e-6，固定向量下应不同）
        assert_ne!(derive_pin(&pk_a, &pk_b), derive_pin(&pk_a, &pk_c));
    }
}
