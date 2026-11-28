//! How to run an HMI beside the emulation, per manufacturer: the steps, and the Windows settings window that goes with them.
//! The bridge never changes a system setting itself; it tells the user what to set and opens the window where they set it.

/// A Windows settings window the bridge can open for the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opener {
    /// Siemens "PG/PC Interface Settings" control panel applet (32-bit, installed with STEP 7 / TIA Portal).
    SiemensPgPc,
    /// Windows network adapters (ncpa.cpl).
    NetworkAdapters,
}

impl Opener {
    pub fn label(self) -> &'static str {
        match self {
            Opener::SiemensPgPc => "Open Set PG/PC Interface",
            Opener::NetworkAdapters => "Open Windows network adapters",
        }
    }

    /// The program and arguments that open the window.
    pub fn command(self) -> (String, Vec<String>) {
        match self {
            Opener::SiemensPgPc => ("C:\\Windows\\SysWOW64\\rundll32.exe".into(), vec!["shell32.dll,Control_RunDLL".into(), "C:\\Windows\\SysWOW64\\s7epatdx.cpl".into()]),
            Opener::NetworkAdapters => ("control.exe".into(), vec!["ncpa.cpl".into()]),
        }
    }

    /// Whether the window exists on this machine (the Siemens applet only after STEP 7 / TIA Portal is installed).
    pub fn available(self) -> bool {
        match self {
            Opener::SiemensPgPc => std::path::Path::new("C:\\Windows\\SysWOW64\\s7epatdx.cpl").exists(),
            Opener::NetworkAdapters => true,
        }
    }

    pub fn open(self) -> Result<(), String> {
        let (prog, args) = self.command();
        std::process::Command::new(prog).args(args).spawn().map(|_| ()).map_err(|e| e.to_string())
    }
}

pub struct Maker {
    pub name: &'static str,
    /// The emulator and HMI product the steps are written for.
    pub products: &'static str,
    pub steps: &'static [&'static str],
    pub note: &'static str,
    /// The window to open for the step that needs a setting (the bridge opens it; the user changes it).
    pub opener: Option<Opener>,
}

pub const INTRO: &str = "The model and the HMI show the same machine only when both are driven by the same PLC (or emulation). The HMI runtime has to run against the emulation, not against nothing. That needs one setting on this PC: the communication interface the engineering tools and the HMI use has to be the emulator's virtual adapter. It is a Windows system setting, so the bridge does not change it: set it yourself, then start the HMI runtime. The bridge opens the right window for you.";

pub const MAKERS: &[Maker] = &[
    Maker {
        name: "Siemens",
        products: "TIA Portal WinCC (Basic / Advanced / Unified runtime) with S7-PLCSIM or PLCSIM Advanced",
        steps: &[
            "Start the PLC emulation and load the program: an S7-PLCSIM Advanced instance (for example PL_Sim) shown green under 'Active PLC Instance(s)', with 'TCP/IP communication with <Local>'. For S7-PLCSIM V18 start the simulator and power on the instance.",
            "Check this PC has an adapter in the instance's address range (run ipconfig): for PLCSIM Advanced it is 169.254.x.x, the same first two numbers as the instance's IP address.",
            "Open Set PG/PC Interface: press the button below, or Windows Control Panel (View by: Large icons) > Set PG/PC Interface (32-bit), or press the Windows key, type s7epatdx.cpl and press Enter.",
            "In the window, tab 'Access Path': set 'Access Point of the Application' to 'S7ONLINE (STEP 7)'. In the list under it select 'PLCSIM.TCPIP.1' (some versions call it 'PLCSIM Virtual Eth. Adapter.TCPIP.1'). 'Interface Parameter Assignment Used' now reads 'PLCSIM.TCPIP.1'. For S7-PLCSIM V18 choose 'PLCSIM' instead.",
            "Press OK (confirm if asked). Cancel changes nothing. If it already reads PLCSIM.TCPIP.1 there is nothing to change.",
            "In TIA Portal check the HMI device has a connection to the PLC in the network view, and the HMI compiles with 0 errors.",
            "Start the HMI runtime: in the project tree select the HMI device, then Online > Simulation > Start. The 'SIMATIC WinCC Runtime Advanced' window opens.",
            "Check it is connected: numbers and lamps move when the PLC moves. If every value shows ###, the link to the PLC is lost: check steps 1 to 5 and the HMI's connection IP address. Ignore the repeating 'Too many tags (Powertags)' box (click Noted).",
            "Drive the machine only from the HMI's buttons or the PLC's own transport commands while the model is shown, so HMI and model follow the same PLC.",
        ],
        note: "Steps 1 to 8 were done and seen on a real TIA Portal V18 + PLCSIM Advanced V5.0 + WinCC Runtime Advanced PC (see Knowledge: Siemens_HMI_With_Model_Connection.md). Names can differ a little in other versions: read the window, not this text. The bridge opens the window; it never changes the setting.",
        opener: Some(Opener::SiemensPgPc),
    },
    Maker {
        name: "Rockwell Automation",
        products: "FactoryTalk View ME / SE with Studio 5000 Logix Emulate or a real controller",
        steps: &[
            "Start the controller emulation in Studio 5000 Logix Emulate and download the project to it.",
            "In RSLinx Classic or FactoryTalk Linx, add a driver for the emulator (Virtual Backplane, or the Ethernet driver of the emulation PC).",
            "In FactoryTalk View, point the HMI's data server or shortcut at the emulated controller's path.",
            "Start the FactoryTalk View runtime (Test Application, or run the SE client). The HMI reads the same tags as the model.",
        ],
        note: "The driver setting is inside RSLinx / FactoryTalk Linx, not in Windows, so there is no window for the bridge to open.",
        opener: Some(Opener::NetworkAdapters),
    },
    Maker {
        name: "Schneider Electric",
        products: "EcoStruxure Machine Expert (Vijeo Designer / Harmony) or Control Expert with their simulators",
        steps: &[
            "Start the simulator of the controller in EcoStruxure Machine Expert or Control Expert and download the application.",
            "Set the communication path of the project to the simulator (its gateway, or the loopback address).",
            "In the HMI project set the equipment address to the same simulator, then start the HMI runtime or its simulator.",
        ],
        note: "Check the simulator's gateway or loopback address in the vendor's documentation for your version.",
        opener: Some(Opener::NetworkAdapters),
    },
    Maker {
        name: "Mitsubishi Electric",
        products: "GX Works3 with GX Simulator3 and GT Designer3 / GT Simulator3",
        steps: &[
            "Start GX Simulator3 from GX Works3 and write the program to it.",
            "In GT Designer3 set the controller connection of the screen project to the simulator (the simulation connection).",
            "Start GT Simulator3. The HMI reads the same devices as the model.",
        ],
        note: "Both simulators run on the one PC; no Windows setting is normally needed.",
        opener: None,
    },
    Maker {
        name: "Omron",
        products: "Sysmac Studio simulation with NA series (Sysmac Studio) HMI simulation",
        steps: &[
            "Run the controller in Sysmac Studio's simulation and transfer the project to it.",
            "In the NA project set the connection to the simulated controller (the Sysmac Studio simulation address).",
            "Start the NA simulation. The HMI reads the same variables as the model.",
        ],
        note: "Use the simulation address shown in Sysmac Studio for your version.",
        opener: Some(Opener::NetworkAdapters),
    },
    Maker {
        name: "Beckhoff",
        products: "TwinCAT 3 with TwinCAT HMI",
        steps: &[
            "Activate the TwinCAT configuration on this PC in run mode (the PC is the controller).",
            "In TwinCAT HMI choose the local AMS route / server as the data source.",
            "Start the TwinCAT HMI server and open the HMI in the browser. It reads the same symbols as the model.",
        ],
        note: "The PC itself is the controller, so there is no adapter to change; the route is set inside TwinCAT.",
        opener: None,
    },
    Maker {
        name: "CODESYS",
        products: "CODESYS Control Win with CODESYS Visualization (WebVisu / TargetVisu)",
        steps: &[
            "Start CODESYS Control Win (the soft PLC) and log in with the project.",
            "Make sure the gateway in CODESYS shows the soft PLC as online.",
            "Start the application and open the visualization (WebVisu in the browser, or the target visualization). It reads the same variables as the model.",
        ],
        note: "Soft PLC and visualization run on the same PC, so no Windows setting is normally needed.",
        opener: None,
    },
];

pub fn maker(name: &str) -> Option<&'static Maker> {
    MAKERS.iter().find(|m| m.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_maker_has_steps_and_siemens_opens_the_pg_pc_applet() {
        assert!(MAKERS.iter().all(|m| !m.steps.is_empty() && !m.products.is_empty() && !m.note.is_empty()));
        assert!(MAKERS.windows(2).all(|w| w[0].name != w[1].name));
        let (prog, args) = Opener::SiemensPgPc.command();
        assert!(prog.ends_with("rundll32.exe") && args[1].ends_with("s7epatdx.cpl"));
        assert_eq!(maker("Siemens").and_then(|m| m.opener), Some(Opener::SiemensPgPc));
        assert!(maker("Nobody").is_none());
    }

    #[test]
    fn the_siemens_steps_name_the_virtual_adapter() {
        let s = maker("Siemens").map(|m| m.steps.join(" ")).unwrap_or_default();
        assert!(s.contains("PLCSIM.TCPIP.1") && s.contains("S7ONLINE") && s.contains("s7epatdx.cpl"));
    }
}
