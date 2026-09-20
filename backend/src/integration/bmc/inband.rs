//! In-band BMC access from the host OS (`ipmitool` over SSH, no LAN credentials).
//!
//! Used during convert/takeover: if TCS does not already have BMC
//! username/password, create an administrator user named `tcs` with a random
//! password and store it encrypted on the machine row.

use crate::integration::ssh::SshClient;
use crate::AppError;

pub const TCS_BMC_USER: &str = "tcs";

/// Parse `ipmitool user list 1` into (id, name). Empty name = unused slot.
pub fn parse_ipmi_user_list(stdout: &str) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    for line in stdout.lines() {
        let t = line.trim();
        if t.is_empty() || t.to_ascii_uppercase().starts_with("ID") {
            continue;
        }
        let mut parts = t.split_whitespace();
        let Some(id) = parts.next().and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        let next = parts.next().unwrap_or("");
        let name = if next.eq_ignore_ascii_case("true") || next.eq_ignore_ascii_case("false") {
            String::new()
        } else {
            next.to_string()
        };
        out.push((id, name));
    }
    out
}

/// Prefer an existing `tcs` account; otherwise the first empty slot with id >= 2.
pub fn pick_tcs_user_slot(users: &[(u32, String)]) -> Option<u32> {
    if let Some((id, _)) = users
        .iter()
        .find(|(_, n)| n.eq_ignore_ascii_case(TCS_BMC_USER))
    {
        return Some(*id);
    }
    users
        .iter()
        .find(|(id, n)| *id >= 2 && n.is_empty())
        .map(|(id, _)| *id)
}

/// 16-char IPMI-safe password (no punctuation; many iLO/iDRAC caps at 16–20).
pub fn random_bmc_password() -> String {
    const A: &[u8] = b"abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    (0..16)
        .map(|_| A[fastrand::usize(0..A.len())] as char)
        .collect()
}

/// Create or reset the in-band `tcs` BMC admin user. Password must be
/// alphanumeric (no shell quoting). Does not log the password.
pub async fn provision_tcs_bmc_user(
    ssh: &SshClient,
    host: &str,
    password: &str,
) -> Result<u32, AppError> {
    let _ = ssh
        .run_capture(host, "modprobe ipmi_devintf 2>/dev/null; modprobe ipmi_si 2>/dev/null; true")
        .await;
    let list = ssh
        .run(host, "ipmitool user list 1")
        .await
        .map_err(|e| AppError::Network(format!("ipmitool user list: {e}")))?;
    let users = parse_ipmi_user_list(&list);
    let id = pick_tcs_user_slot(&users).ok_or_else(|| {
        AppError::Network("no free BMC user slot for tcs (id>=2 empty, or existing tcs)".into())
    })?;
    // Name may already be tcs; still reset password and privileges.
    let _ = ssh
        .run_capture(host, &format!("ipmitool user set name {id} {TCS_BMC_USER}"))
        .await;
    // Privilege 4 = ADMINISTRATOR. Password is alphanumeric (no quoting).
    let cmd = format!(
        "ipmitool user set password {id} {password} && \
         ipmitool channel setaccess 1 {id} ipmi=on link=on callin=on privilege=4 && \
         ipmitool user priv {id} 4 1 && \
         ipmitool user enable {id}"
    );
    ssh.run(host, &cmd)
        .await
        .map_err(|e| AppError::Network(format!("ipmitool create user {TCS_BMC_USER}: {e}")))?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_idrac_style_list() {
        let t = "\
ID  Name             Callin  Link Auth  IPMI Msg   Channel Priv Limit
1                    true    false      false      NO ACCESS
2   ADMIN            true    true       true       ADMINISTRATOR
3                    true    false      false      NO ACCESS
4   root             true    true       true       ADMINISTRATOR
";
        let u = parse_ipmi_user_list(t);
        assert_eq!(u[0], (1, "".into()));
        assert_eq!(u[1], (2, "ADMIN".into()));
        assert_eq!(u[2], (3, "".into()));
        assert_eq!(pick_tcs_user_slot(&u), Some(3));
    }

    #[test]
    fn reuse_existing_tcs_slot() {
        let u = vec![(2, "ADMIN".into()), (3, "tcs".into()), (4, "".into())];
        assert_eq!(pick_tcs_user_slot(&u), Some(3));
    }

    #[test]
    fn password_is_16_alnum() {
        let p = random_bmc_password();
        assert_eq!(p.len(), 16);
        assert!(p.chars().all(|c| c.is_ascii_alphanumeric()));
    }
}
