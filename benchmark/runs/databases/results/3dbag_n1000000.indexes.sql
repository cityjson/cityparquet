-- cjdb: the one index genuinely missing from cjdb's own defaults
-- (added by this harness's ingest() -- see sql_cjdb.index_ddl()'s
-- docstring for why every other index is already cjdb's own):
CREATE INDEX IF NOT EXISTS ix_co_object_id ON cjdb.city_object (object_id);
-- 3dcitydb: this harness adds nothing -- citydb-tool's own import
-- already creates every index the scenario queries need (see
-- sql_citydb.index_ddl()'s docstring). An empty list here is the
-- correct, verified answer, not an omission.

-- cjdb: FULL live `pg_indexes` dump (I7) --
CREATE UNIQUE INDEX city_object_cj_metadata_id_object_id_key ON cjdb.city_object USING btree (cj_metadata_id, object_id);
CREATE INDEX city_object_ground_gix ON cjdb.city_object USING gist (ground_geometry);
CREATE UNIQUE INDEX city_object_pkey ON cjdb.city_object USING btree (id);
CREATE INDEX city_object_type_idx ON cjdb.city_object USING btree (type);
CREATE INDEX idx_city_object_ground_geometry ON cjdb.city_object USING gist (ground_geometry);
CREATE INDEX ix_co_object_id ON cjdb.city_object USING btree (object_id);
CREATE INDEX lod ON cjdb.city_object USING gin (geometry);
CREATE INDEX city_object_relationships_child_idx ON cjdb.city_object_relationships USING btree (child_id);
CREATE UNIQUE INDEX city_object_relationships_parent_id_child_id_key ON cjdb.city_object_relationships USING btree (parent_id, child_id);
CREATE INDEX city_object_relationships_parent_idx ON cjdb.city_object_relationships USING btree (parent_id);
CREATE UNIQUE INDEX city_object_relationships_pkey ON cjdb.city_object_relationships USING btree (id);
CREATE INDEX cj_metadata_gix ON cjdb.cj_metadata USING gist (bbox);
CREATE UNIQUE INDEX cj_metadata_pkey ON cjdb.cj_metadata USING btree (id);
CREATE INDEX cj_metadata_source_file_idx ON cjdb.cj_metadata USING hash (source_file);
CREATE INDEX idx_cj_metadata_bbox ON cjdb.cj_metadata USING gist (bbox);

-- 3dcitydb: FULL live `pg_indexes` dump (I7) --
CREATE UNIQUE INDEX address_pk ON citydb.address USING btree (id);
CREATE UNIQUE INDEX ade_pk ON citydb.ade USING btree (id);
CREATE INDEX appear_to_surface_data_fkx1 ON citydb.appear_to_surface_data USING btree (surface_data_id);
CREATE INDEX appear_to_surface_data_fkx2 ON citydb.appear_to_surface_data USING btree (appearance_id);
CREATE UNIQUE INDEX appear_to_surface_data_pk ON citydb.appear_to_surface_data USING btree (id);
CREATE INDEX appearance_feature_fkx ON citydb.appearance USING btree (feature_id);
CREATE INDEX appearance_implicit_geom_fkx ON citydb.appearance USING btree (implicit_geometry_id);
CREATE UNIQUE INDEX appearance_pk ON citydb.appearance USING btree (id);
CREATE INDEX appearance_theme_inx ON citydb.appearance USING btree (theme);
CREATE INDEX codelist_codelist_type_inx ON citydb.codelist USING btree (codelist_type);
CREATE UNIQUE INDEX codelist_pk ON citydb.codelist USING btree (id);
CREATE INDEX codelist_entry_codelist_fkx ON citydb.codelist_entry USING btree (codelist_id);
CREATE UNIQUE INDEX codelist_entry_pk ON citydb.codelist_entry USING btree (id);
CREATE UNIQUE INDEX database_srs_pk ON citydb.database_srs USING btree (srid);
CREATE UNIQUE INDEX datatype_pk ON citydb.datatype USING btree (id);
CREATE INDEX datatype_supertype_fkx ON citydb.datatype USING btree (supertype_id);
CREATE INDEX feature_creation_date_inx ON citydb.feature USING btree (creation_date);
CREATE INDEX feature_envelope_spx ON citydb.feature USING gist (envelope);
CREATE INDEX feature_identifier_inx ON citydb.feature USING btree (identifier, identifier_codespace);
CREATE INDEX feature_objectclass_inx ON citydb.feature USING btree (objectclass_id);
CREATE INDEX feature_objectid_inx ON citydb.feature USING btree (objectid);
CREATE UNIQUE INDEX feature_pk ON citydb.feature USING btree (id);
CREATE INDEX feature_termination_date_inx ON citydb.feature USING btree (termination_date);
CREATE INDEX feature_valid_from_inx ON citydb.feature USING btree (valid_from);
CREATE INDEX feature_valid_to_inx ON citydb.feature USING btree (valid_to);
CREATE INDEX geometry_data_feature_fkx ON citydb.geometry_data USING btree (feature_id);
CREATE UNIQUE INDEX geometry_data_pk ON citydb.geometry_data USING btree (id);
CREATE INDEX geometry_data_spx ON citydb.geometry_data USING gist (geometry);
CREATE INDEX implicit_geometry_fkx ON citydb.implicit_geometry USING btree (relative_geometry_id);
CREATE INDEX implicit_geometry_objectid_inx ON citydb.implicit_geometry USING btree (objectid);
CREATE UNIQUE INDEX implicit_geometry_pk ON citydb.implicit_geometry USING btree (id);
CREATE UNIQUE INDEX namespace_pk ON citydb.namespace USING btree (id);
CREATE UNIQUE INDEX objectclass_pk ON citydb.objectclass USING btree (id);
CREATE INDEX objectclass_superclass_fkx ON citydb.objectclass USING btree (superclass_id);
CREATE INDEX property_feature_fkx ON citydb.property USING btree (feature_id);
CREATE INDEX property_name_inx ON citydb.property USING btree (name);
CREATE INDEX property_namespace_inx ON citydb.property USING btree (namespace_id);
CREATE INDEX property_parent_fkx ON citydb.property USING btree (parent_id);
CREATE UNIQUE INDEX property_pk ON citydb.property USING btree (id);
CREATE INDEX property_val_address_fkx ON citydb.property USING btree (val_address_id);
CREATE INDEX property_val_appearance_fkx ON citydb.property USING btree (val_appearance_id);
CREATE INDEX property_val_date_inx ON citydb.property USING btree (val_timestamp) WHERE (val_timestamp IS NOT NULL);
CREATE INDEX property_val_double_inx ON citydb.property USING btree (val_double) WHERE (val_double IS NOT NULL);
CREATE INDEX property_val_feature_fkx ON citydb.property USING btree (val_feature_id);
CREATE INDEX property_val_geometry_fkx ON citydb.property USING btree (val_geometry_id);
CREATE INDEX property_val_implicitgeom_fkx ON citydb.property USING btree (val_implicitgeom_id);
CREATE INDEX property_val_int_inx ON citydb.property USING btree (val_int) WHERE (val_int IS NOT NULL);
CREATE INDEX property_val_lod_inx ON citydb.property USING btree (val_lod);
CREATE INDEX property_val_relation_type_inx ON citydb.property USING btree (val_relation_type);
CREATE INDEX property_val_string_inx ON citydb.property USING btree (val_string) WHERE (val_string IS NOT NULL);
CREATE INDEX property_val_uom_inx ON citydb.property USING btree (val_uom) WHERE (val_uom IS NOT NULL);
CREATE INDEX property_val_uri_inx ON citydb.property USING btree (val_uri) WHERE (val_uri IS NOT NULL);
CREATE INDEX surface_data_objclass_fkx ON citydb.surface_data USING btree (objectclass_id);
CREATE UNIQUE INDEX surface_data_pk ON citydb.surface_data USING btree (id);
CREATE INDEX surface_data_tex_image_fkx ON citydb.surface_data USING btree (tex_image_id);
CREATE INDEX surface_data_mapping_fkx1 ON citydb.surface_data_mapping USING btree (geometry_data_id);
CREATE INDEX surface_data_mapping_fkx2 ON citydb.surface_data_mapping USING btree (surface_data_id);
CREATE UNIQUE INDEX surface_data_mapping_pk ON citydb.surface_data_mapping USING btree (geometry_data_id, surface_data_id);
CREATE UNIQUE INDEX tex_image_pk ON citydb.tex_image USING btree (id);
