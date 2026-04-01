use bitflags::bitflags;

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct WasmPackagePermission: i64 {
        const Owner      = 0b00000000_00000001;
        const Maintainer = 0b00000000_00000010;
        const User       = 0b00000000_00000100;
        const Buyer      = 0b00000000_00001000;
    }
}

impl WasmPackagePermission {
    pub fn has_permission(self, required: Self) -> bool {
        if self.contains(Self::Owner) {
            return true;
        }
        if self.contains(Self::Maintainer) && !required.intersects(Self::Owner) {
            return true;
        }
        self.contains(required)
    }

    pub fn can_manage_level(self, target: Self) -> bool {
        if target.contains(Self::Buyer) {
            return false;
        }
        if self.contains(Self::Owner) {
            return true;
        }
        self.contains(Self::Maintainer) && target == Self::User
    }

    pub fn is_buyer(self) -> bool {
        self.contains(Self::Buyer)
    }
}
