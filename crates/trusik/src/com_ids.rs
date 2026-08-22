use windows::core::GUID;

pub const IID_IUNKNOWN: GUID = GUID {
    data1: 0x00000000,
    data2: 0x0000,
    data3: 0x0000,
    data4: [0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};

pub const IID_IDIRECTINPUT8A: GUID = GUID {
    data1: 0xBF798030,
    data2: 0x483A,
    data3: 0x4DA2,
    data4: [0xAA, 0x99, 0x5D, 0x64, 0xED, 0x36, 0x97, 0x00],
};

pub const IID_IDIRECTINPUT8W: GUID = GUID {
    data1: 0xBF798031,
    data2: 0x483A,
    data3: 0x4DA2,
    data4: [0xAA, 0x99, 0x5D, 0x64, 0xED, 0x36, 0x97, 0x00],
};

pub const IID_IDIRECTINPUTDEVICE8A: GUID = GUID {
    data1: 0x54D41080,
    data2: 0xDC15,
    data3: 0x4833,
    data4: [0xA4, 0x1B, 0x74, 0x8F, 0x73, 0xA3, 0x81, 0x79],
};

pub const IID_IDIRECTINPUTDEVICE8W: GUID = GUID {
    data1: 0x54D41081,
    data2: 0xDC15,
    data3: 0x4833,
    data4: [0xA4, 0x1B, 0x74, 0x8F, 0x73, 0xA3, 0x81, 0x79],
};

pub fn is_direct_input_8(iid: GUID) -> bool {
    matches!(iid, IID_IDIRECTINPUT8A | IID_IDIRECTINPUT8W)
}

pub fn device_iid_for(direct_input_iid: GUID) -> Option<GUID> {
    if direct_input_iid == IID_IDIRECTINPUT8A {
        Some(IID_IDIRECTINPUTDEVICE8A)
    } else if direct_input_iid == IID_IDIRECTINPUT8W {
        Some(IID_IDIRECTINPUTDEVICE8W)
    } else {
        None
    }
}
