export type ConflictSide={source_id:string|null;fact_version:number|null;path:string;location:string;quote:string;value:string};
export type SourceConflict={id:string;topic:string;category:'date_difference'|'contribution_difference';reason:string;left:ConflictSide;right:ConflictSide};
