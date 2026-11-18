#![no_std]
#![no_main]

#[dunit::main]
fn main() -> dunit::Result<()> {
    match dunit_net_core::diagnostic::run() {
        Ok(report) => {
            dunit::println!(
                "[NET-CORE] logical-peer echo={} ARP={} rejected={} checked",
                report.echo_replies,
                report.arp_requests,
                report.rejected
            )?;
            if report.echo_replies != 3 || report.arp_requests != 5 || report.rejected != 5 {
                dunit::println!("[NET-CORE] FAIL counts")?;
                return Err(dunit::Error(dunit::errno::EINVAL));
            }
            let routed = match dunit_net_core::diagnostic::run_routed() {
                Ok(report) => report,
                Err(error) => {
                    dunit::println!("[NET-CORE] FAIL routed {:?}", error)?;
                    return Err(dunit::Error(dunit::errno::EINVAL));
                }
            };
            if routed.echo_replies != 2 || routed.arp_requests != 1 || routed.rejected != 3 {
                dunit::println!("[NET-CORE] FAIL routed counts")?;
                return Err(dunit::Error(dunit::errno::EINVAL));
            }
            dunit::println!(
                "[NET-CORE] routed-peer echo={} gateway-ARP={} rejected={} checked",
                routed.echo_replies,
                routed.arp_requests,
                routed.rejected
            )?;
            dunit::println!("[NET-CORE] codec logical-peer OK; NIC service gate open")?;
            Ok(())
        }
        Err(error) => {
            dunit::println!("[NET-CORE] FAIL {:?}", error)?;
            Err(dunit::Error(dunit::errno::EINVAL))
        }
    }
}
