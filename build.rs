use std::{fs, path::Path};

const ICON_ID: u16 = 1;

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;

fn main() {
	let root = Path::new(env!("CARGO_MANIFEST_DIR"));
	let manifest = root.join("knob.manifest");
	let icon = root.join("assets").join("knob.ico");

	println!("cargo:rerun-if-changed=knob.manifest");
	println!("cargo:rerun-if-changed=assets/knob.ico");

	println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
	println!("cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}", manifest.display());
	println!("cargo:rustc-link-arg-bins=/MANIFESTUAC:NO");

	let out = Path::new(&std::env::var("OUT_DIR").expect("cargo sets OUT_DIR")).join("knob.res");
	let ico = fs::read(&icon).expect("assets/knob.ico is missing");

	fs::write(&out, build_res(&ico)).expect("failed to write the resource file");

	println!("cargo:rustc-link-arg-bins={}", out.display());
}

fn build_res(ico: &[u8]) -> Vec<u8> {
	let count = u16::from_le_bytes([ico[4], ico[5]]);

	let mut res = Vec::new();
	res.extend_from_slice(&[0, 0, 0, 0, 32, 0, 0, 0]);
	res.extend_from_slice(&[0xFF, 0xFF, 0, 0, 0xFF, 0xFF, 0, 0]);
	res.extend_from_slice(&[0; 16]);

	let mut group = Vec::new();
	group.extend_from_slice(&[0, 0]);
	group.extend_from_slice(&1_u16.to_le_bytes());
	group.extend_from_slice(&count.to_le_bytes());

	for index in 0..count {
		let entry = 6 + 16 * usize::from(index);
		let header = &ico[entry..entry + 16];

		let size = u32::from_le_bytes([header[8], header[9], header[10], header[11]]);
		let offset = u32::from_le_bytes([header[12], header[13], header[14], header[15]]);

		let start = offset as usize;
		let image = &ico[start..start + size as usize];

		group.extend_from_slice(&header[..12]);
		group.extend_from_slice(&(index + 1).to_le_bytes());

		append(&mut res, RT_ICON, index + 1, image);
	}

	append(&mut res, RT_GROUP_ICON, ICON_ID, &group);

	res
}

fn append(res: &mut Vec<u8>, kind: u16, id: u16, data: &[u8]) {
	res.extend_from_slice(&(data.len() as u32).to_le_bytes());
	res.extend_from_slice(&32_u32.to_le_bytes());

	for ordinal in [kind, id] {
		res.extend_from_slice(&0xFFFF_u16.to_le_bytes());
		res.extend_from_slice(&ordinal.to_le_bytes());
	}

	res.extend_from_slice(&0_u32.to_le_bytes());
	res.extend_from_slice(&0x1010_u16.to_le_bytes());
	res.extend_from_slice(&0x0409_u16.to_le_bytes());
	res.extend_from_slice(&0_u32.to_le_bytes());
	res.extend_from_slice(&0_u32.to_le_bytes());

	res.extend_from_slice(data);
	res.resize(res.len().next_multiple_of(4), 0);
}
