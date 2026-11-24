use crate::pathfinder::Operation;

pub const ARE_ENTITIES_IN_LIBRARY: Operation = Operation::query(
    "areEntitiesInLibrary",
    "134337999233cc6fdd6b1e6dbf94841409f04a946c5c7b744b09ba0dfe5a85ed",
);
pub const ADD_TO_LIBRARY: Operation = Operation::mutation(
    "addToLibrary",
    "896ebcb47815681340860d121cb5d494e157e2a78d3950385cd54e0393c67148",
);
pub const REMOVE_FROM_LIBRARY: Operation = Operation::mutation(
    "removeFromLibrary",
    "896ebcb47815681340860d121cb5d494e157e2a78d3950385cd54e0393c67148",
);
