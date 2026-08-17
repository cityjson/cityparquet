<?xml version="1.0" encoding="UTF-8"?>
<!--
  Test fragment from Japan's PLATEAU 3D City Models (Tokyo, Tachikawa-shi;
  13202_tachikawa-shi_pref_2023_citygml_2_op), CityGML 2.0 module `trk`
  (Track / footpaths), tile 53394343_trk_6697_op.gml (547 KB, 11 Tracks).
  Licence: CC BY 4.0 (Project PLATEAU, MLIT Japan). The source URL is the
  `trk` entry in bench/catalogue_benchmark_urls.txt.

  Root CityModel + gml:Envelope + THREE of the tile's eleven
  cityObjectMembers (the three smallest, kept in document order), verbatim.

  Every member is a `tran:Track` — a 1st-level CityObject type this reader
  does not map. A reader that silently skips unmapped members therefore
  reports 0 objects for a document that plainly holds three, in a fraction of
  the time a real read would take. That silent zero is exactly what the
  readbench `citygml` runner's skipped-member guard refuses.
-->
<core:CityModel xmlns:grp="http://www.opengis.net/citygml/cityobjectgroup/2.0" xmlns:core="http://www.opengis.net/citygml/2.0" xmlns:pbase="http://www.opengis.net/citygml/profiles/base/2.0" xmlns:smil20lang="http://www.w3.org/2001/SMIL20/Language" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:smil20="http://www.w3.org/2001/SMIL20/" xmlns:bldg="http://www.opengis.net/citygml/building/2.0" xmlns:uro="https://www.geospatial.jp/iur/uro/3.1" xmlns:xAL="urn:oasis:names:tc:ciq:xsdschema:xAL:2.0" xmlns:luse="http://www.opengis.net/citygml/landuse/2.0" xmlns:gen="http://www.opengis.net/citygml/generics/2.0" xmlns:dem="http://www.opengis.net/citygml/relief/2.0" xmlns:app="http://www.opengis.net/citygml/appearance/2.0" xmlns:tex="http://www.opengis.net/citygml/texturedsurface/2.0" xmlns:xlink="http://www.w3.org/1999/xlink" xmlns:tun="http://www.opengis.net/citygml/tunnel/2.0" xmlns:sch="http://www.ascc.net/xml/schematron" xmlns:veg="http://www.opengis.net/citygml/vegetation/2.0" xmlns:frn="http://www.opengis.net/citygml/cityfurniture/2.0" xmlns:gml="http://www.opengis.net/gml" xmlns:tran="http://www.opengis.net/citygml/transportation/2.0" xmlns:wtr="http://www.opengis.net/citygml/waterbody/2.0" xmlns:brid="http://www.opengis.net/citygml/bridge/2.0" xsi:schemaLocation="https://www.geospatial.jp/iur/uro/3.1 ../../schemas/iur/uro/3.1/urbanObject.xsd http://www.opengis.net/citygml/2.0 http://schemas.opengis.net/citygml/2.0/cityGMLBase.xsd http://www.opengis.net/citygml/landuse/2.0 http://schemas.opengis.net/citygml/landuse/2.0/landUse.xsd http://www.opengis.net/citygml/building/2.0 http://schemas.opengis.net/citygml/building/2.0/building.xsd http://www.opengis.net/citygml/transportation/2.0 http://schemas.opengis.net/citygml/transportation/2.0/transportation.xsd http://www.opengis.net/citygml/generics/2.0 http://schemas.opengis.net/citygml/generics/2.0/generics.xsd http://www.opengis.net/citygml/cityobjectgroup/2.0 http://schemas.opengis.net/citygml/cityobjectgroup/2.0/cityObjectGroup.xsd http://www.opengis.net/gml http://schemas.opengis.net/gml/3.1.1/base/gml.xsd http://www.opengis.net/citygml/appearance/2.0 http://schemas.opengis.net/citygml/appearance/2.0/appearance.xsd">
	<gml:boundedBy>
		<gml:Envelope srsName="http://www.opengis.net/def/crs/EPSG/0/6697" srsDimension="3">
			<gml:lowerCorner>35.70035422882568 139.4123603865462 0</gml:lowerCorner>
			<gml:upperCorner>35.705308825703355 139.41296889996448 86.73200308224493</gml:upperCorner>
		</gml:Envelope>
	</gml:boundedBy>
	<core:cityObjectMember>
		<tran:Track gml:id="trk_68254a26-5aa7-4390-8cc8-c12d571e163f">
			<core:creationDate>2024-03-15</core:creationDate>
			<tran:class codeSpace="../../codelists/TransportationComplex_class.xml">1020</tran:class>
			<tran:function codeSpace="../../codelists/Track_function.xml">2</tran:function>
			<tran:trafficArea>
				<tran:TrafficArea gml:id="tfa_a877d325-6f92-4520-860f-3ceb09d02c11">
					<tran:function codeSpace="../../codelists/TrafficArea_function.xml">2000</tran:function>
					<tran:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_76aad17a-4f43-49ec-b378-4024ee93225c">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.702812168326666 139.41241100951459 0 35.70280596907775 139.41286137111913 0 35.70283815223804 139.4128613922079 0 35.70283814243507 139.41285987611104 0 35.702844312273015 139.41241167377598 0 35.702812168326666 139.41241100951459 0</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</tran:lod2MultiSurface>
				</tran:TrafficArea>
			</tran:trafficArea>
			<tran:trafficArea>
				<tran:TrafficArea gml:id="tfa_1d800aa1-dcd4-4a82-80be-05785f644295">
					<tran:function codeSpace="../../codelists/TrafficArea_function.xml">2000</tran:function>
					<tran:lod3MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_97241980-e71c-4b27-997c-92284910274e">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.70283814243507 139.41285987611104 85.41328052902819 35.702834812827525 139.41255191216646 85.40770207801206 35.70283221198436 139.4127103920511 85.40764148155453 35.70283814243507 139.41285987611104 85.41328052902819</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_9185c560-c0e4-4014-b79e-4e699b541fbe">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.702834812827525 139.41255191216646 85.40770207801206 35.70283814243507 139.41285987611104 85.41328052902819 35.702844312100765 139.41241167415495 85.41264618851811 35.702834812827525 139.41255191216646 85.40770207801206</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_0ec2d50c-396e-4fd6-a2bd-129fb57896de">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.70282030183768 139.4125515620266 85.39783916936283 35.702844312100765 139.41241167415495 85.41264618851811 35.70281216848164 139.41241100918558 85.39079982920659 35.70282030183768 139.4125515620266 85.39783916936283</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_cad3c086-fa08-4dfa-a50d-1fb6aafc706b">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.702844312100765 139.41241167415495 85.41264618851811 35.70282030183768 139.4125515620266 85.39783916936283 35.702834812827525 139.41255191216646 85.40770207801206 35.702844312100765 139.41241167415495 85.41264618851811</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_d7149629-6dc9-4a3d-b410-c1593ea28c07">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.70283814243507 139.41285987611104 85.41328052902819 35.70283221198436 139.4127103920511 85.40764148155453 35.70280596907773 139.41286137111913 85.39143722697989 35.70283814243507 139.41285987611104 85.41328052902819</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_685a4aad-dd6c-442b-b9db-dbd62d76a284">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.70280596907773 139.41286137111913 85.39143722697989 35.70283221198436 139.4127103920511 85.40764148155453 35.70280804963917 139.4127102402978 85.39122333134272 35.70280596907773 139.41286137111913 85.39143722697989</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_243f9efb-78d3-4c66-a32a-ca6c295bd89a">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.70281023724193 139.41255131917512 85.39099841009157 35.70282030183768 139.4125515620266 85.39783916936283 35.70281216848164 139.41241100918558 85.39079982920659 35.70281023724193 139.41255131917512 85.39099841009157</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</tran:lod3MultiSurface>
				</tran:TrafficArea>
			</tran:trafficArea>
			<tran:auxiliaryTrafficArea>
				<tran:AuxiliaryTrafficArea gml:id="atr_a27a7d0d-6738-4f58-9eb4-d54728442949">
					<tran:function codeSpace="../../codelists/AuxiliaryTrafficArea_function.xml">3000</tran:function>
					<tran:lod3MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_83769380-99d0-4a04-8e5f-4e36bbccb38a">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.70282030183768 139.4125515620266 85.39783916936283 35.70283221198436 139.4127103920511 85.40764148155453 35.702834812827525 139.41255191216646 85.40770207801206 35.70282030183768 139.4125515620266 85.39783916936283</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_0f65d10e-f0ea-44a1-9848-c5de91b7a3d2">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.70283221198436 139.4127103920511 85.40764148155453 35.70282030183768 139.4125515620266 85.39783916936283 35.70280804963917 139.4127102402978 85.39122333134272 35.70283221198436 139.4127103920511 85.40764148155453</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_5b6d4c24-b26c-4911-a981-851c816bf7f0">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.70280804963917 139.4127102402978 85.39122333134272 35.70282030183768 139.4125515620266 85.39783916936283 35.70281023724193 139.41255131917512 85.39099841009157 35.70280804963917 139.4127102402978 85.39122333134272</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</tran:lod3MultiSurface>
				</tran:AuxiliaryTrafficArea>
			</tran:auxiliaryTrafficArea>
			<tran:lod1MultiSurface>
				<gml:MultiSurface>
					<gml:surfaceMember>
						<gml:Polygon>
							<gml:exterior>
								<gml:LinearRing>
									<gml:posList>35.702812168326666 139.41241100951459 0 35.70280596907775 139.41286137111913 0 35.70283815223804 139.4128613922079 0 35.70283814243507 139.41285987611104 0 35.702844312273015 139.41241167377598 0 35.702812168326666 139.41241100951459 0</gml:posList>
								</gml:LinearRing>
							</gml:exterior>
						</gml:Polygon>
					</gml:surfaceMember>
				</gml:MultiSurface>
			</tran:lod1MultiSurface>
			<tran:lod2MultiSurface>
				<gml:MultiSurface>
					<gml:surfaceMember xlink:href="#poly_76aad17a-4f43-49ec-b378-4024ee93225c"></gml:surfaceMember>
				</gml:MultiSurface>
			</tran:lod2MultiSurface>
			<tran:lod3MultiSurface>
				<gml:MultiSurface>
					<gml:surfaceMember xlink:href="#poly_97241980-e71c-4b27-997c-92284910274e"></gml:surfaceMember>
					<gml:surfaceMember xlink:href="#poly_9185c560-c0e4-4014-b79e-4e699b541fbe"></gml:surfaceMember>
					<gml:surfaceMember xlink:href="#poly_0ec2d50c-396e-4fd6-a2bd-129fb57896de"></gml:surfaceMember>
					<gml:surfaceMember xlink:href="#poly_cad3c086-fa08-4dfa-a50d-1fb6aafc706b"></gml:surfaceMember>
					<gml:surfaceMember xlink:href="#poly_d7149629-6dc9-4a3d-b410-c1593ea28c07"></gml:surfaceMember>
					<gml:surfaceMember xlink:href="#poly_685a4aad-dd6c-442b-b9db-dbd62d76a284"></gml:surfaceMember>
					<gml:surfaceMember xlink:href="#poly_243f9efb-78d3-4c66-a32a-ca6c295bd89a"></gml:surfaceMember>
					<gml:surfaceMember xlink:href="#poly_83769380-99d0-4a04-8e5f-4e36bbccb38a"></gml:surfaceMember>
					<gml:surfaceMember xlink:href="#poly_0f65d10e-f0ea-44a1-9848-c5de91b7a3d2"></gml:surfaceMember>
					<gml:surfaceMember xlink:href="#poly_5b6d4c24-b26c-4911-a981-851c816bf7f0"></gml:surfaceMember>
				</gml:MultiSurface>
			</tran:lod3MultiSurface>
			<uro:tranDataQualityAttribute>
				<uro:DataQualityAttribute>
					<uro:geometrySrcDescLod1 codeSpace="../../codelists/DataQualityAttribute_geometrySrcDesc.xml">000</uro:geometrySrcDescLod1>
					<uro:geometrySrcDescLod2 codeSpace="../../codelists/DataQualityAttribute_geometrySrcDesc.xml">000</uro:geometrySrcDescLod2>
					<uro:geometrySrcDescLod3 codeSpace="../../codelists/DataQualityAttribute_geometrySrcDesc.xml">000</uro:geometrySrcDescLod3>
					<uro:thematicSrcDesc codeSpace="../../codelists/DataQualityAttribute_thematicSrcDesc.xml">000</uro:thematicSrcDesc>
					<uro:thematicSrcDesc codeSpace="../../codelists/DataQualityAttribute_thematicSrcDesc.xml">023</uro:thematicSrcDesc>
					<uro:lodType codeSpace="../../codelists/Road_lodType.xml">3.0</uro:lodType>
					<uro:publicSurveyDataQualityAttribute>
						<uro:PublicSurveyDataQualityAttribute>
							<uro:srcScaleLod1 codeSpace="../../codelists/PublicSurveyDataQualityAttribute_srcScale.xml">1</uro:srcScaleLod1>
							<uro:srcScaleLod2 codeSpace="../../codelists/PublicSurveyDataQualityAttribute_srcScale.xml">1</uro:srcScaleLod2>
							<uro:srcScaleLod3 codeSpace="../../codelists/PublicSurveyDataQualityAttribute_srcScale.xml">2</uro:srcScaleLod3>
							<uro:publicSurveySrcDescLod1 codeSpace="../../codelists/PublicSurveyDataQualityAttribute_publicSurveySrcDesc.xml">023</uro:publicSurveySrcDescLod1>
							<uro:publicSurveySrcDescLod2 codeSpace="../../codelists/PublicSurveyDataQualityAttribute_publicSurveySrcDesc.xml">023</uro:publicSurveySrcDescLod2>
							<uro:publicSurveySrcDescLod3 codeSpace="../../codelists/PublicSurveyDataQualityAttribute_publicSurveySrcDesc.xml">011</uro:publicSurveySrcDescLod3>
						</uro:PublicSurveyDataQualityAttribute>
					</uro:publicSurveyDataQualityAttribute>
				</uro:DataQualityAttribute>
			</uro:tranDataQualityAttribute>
		</tran:Track>
	</core:cityObjectMember>
	<core:cityObjectMember>
		<tran:Track gml:id="trk_a08218d6-ae9d-4b9a-bdc9-54d959efaf28">
			<core:creationDate>2024-03-15</core:creationDate>
			<tran:class codeSpace="../../codelists/TransportationComplex_class.xml">1020</tran:class>
			<tran:function codeSpace="../../codelists/Track_function.xml">2</tran:function>
			<tran:trafficArea>
				<tran:TrafficArea gml:id="tfa_f920efed-3608-4a1f-aaee-b4d7d1893d07">
					<tran:function codeSpace="../../codelists/TrafficArea_function.xml">2000</tran:function>
					<tran:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_52d9757c-f4b5-480d-8540-02e3f5a2eeca">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.70195579202734 139.41239331468128 0 35.70192875208404 139.41239356731694 0 35.701927859309876 139.41280268097327 0 35.70193270577296 139.4128026367376 0 35.70195489805868 139.4128031180346 0 35.70195579202734 139.41239331468128 0</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</tran:lod2MultiSurface>
				</tran:TrafficArea>
			</tran:trafficArea>
			<tran:trafficArea>
				<tran:TrafficArea gml:id="tfa_fabcfa4f-4323-48af-a0da-0c4da4d055c4">
					<tran:function codeSpace="../../codelists/TrafficArea_function.xml">2000</tran:function>
					<tran:lod3MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_3a868b04-d86b-4570-b61a-efe8ef38f5fa">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.70195148762501 139.41269161814984 85.05262803721341 35.70192445034574 139.4126916859769 85.03860615934568 35.701949880104955 139.4128030092731 85.05125113502068 35.70195148762501 139.41269161814984 85.05262803721341</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_97542112-79ff-42e8-8f43-e274fe4aeb24">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.701949880104955 139.4128030092731 85.05125113502068 35.70192445034574 139.4126916859769 85.03860615934568 35.70192284786936 139.41280272755355 85.03723357739806 35.701949880104955 139.4128030092731 85.05125113502068</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</tran:lod3MultiSurface>
				</tran:TrafficArea>
			</tran:trafficArea>
			<tran:trafficArea>
				<tran:TrafficArea gml:id="tfa_90f092fe-aa04-4a93-97bd-c12a197c75ae">
					<tran:function codeSpace="../../codelists/TrafficArea_function.xml">2000</tran:function>
					<tran:lod3MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_6dcb8875-dc3e-4450-a55f-ef3e6a69683a">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.70195302630226 139.41253035568477 85.05421254891377 35.70192875208404 139.41239356731694 85.04229119588999 35.701926778415356 139.4125303526753 85.04060039588052 35.70195302630226 139.41253035568477 85.05421254891377</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_d4e95347-2d7b-42c0-8317-f3b44cca3ace">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.70192875208404 139.41239356731694 85.04229119588999 35.70195302630226 139.41253035568477 85.05421254891377 35.70195579202734 139.41239331468128 85.05631535685565 35.70192875208404 139.41239356731694 85.04229119588999</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_5cb10ca1-dc98-4808-ba42-3bdd90d2993d">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.70195579202734 139.41239331468128 85.05631535685565 35.70195302630226 139.41253035568477 85.05421254891377 35.70195378900306 139.41253213454527 85.05459940899092 35.70195579202734 139.41239331468128 85.05631535685565</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</tran:lod3MultiSurface>
				</tran:TrafficArea>
			</tran:trafficArea>
			<tran:auxiliaryTrafficArea>
				<tran:AuxiliaryTrafficArea gml:id="atr_9061dc68-7cb0-46e0-b864-d435d88e16a9">
					<tran:function codeSpace="../../codelists/AuxiliaryTrafficArea_function.xml">3000</tran:function>
					<tran:lod3MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_484b3ab3-b00e-4ad3-8d15-d24aecf9a482">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.70195378900306 139.41253213454527 85.05459940899092 35.70192445034574 139.4126916859769 85.03860615934568 35.70195148762501 139.41269161814984 85.05262803721341 35.70195378900306 139.41253213454527 85.05459940899092</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_939ecb7b-e781-4838-b0ab-58be46e55604">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.70192445034574 139.4126916859769 85.03860615934568 35.70195378900306 139.41253213454527 85.05459940899092 35.701926778415356 139.4125303526753 85.04060039588052 35.70192445034574 139.4126916859769 85.03860615934568</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_2a9f3656-b53e-43b3-8360-86dc9f03fcad">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.701926778415356 139.4125303526753 85.04060039588052 35.70195378900306 139.41253213454527 85.05459940899092 35.70195302630226 139.41253035568477 85.05421254891368 35.701926778415356 139.4125303526753 85.04060039588052</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</tran:lod3MultiSurface>
				</tran:AuxiliaryTrafficArea>
			</tran:auxiliaryTrafficArea>
			<tran:lod1MultiSurface>
				<gml:MultiSurface>
					<gml:surfaceMember>
						<gml:Polygon>
							<gml:exterior>
								<gml:LinearRing>
									<gml:posList>35.70195579202734 139.41239331468128 0 35.70192875208404 139.41239356731694 0 35.701927859309876 139.41280268097327 0 35.70193270577296 139.4128026367376 0 35.70195489805868 139.4128031180346 0 35.70195579202734 139.41239331468128 0</gml:posList>
								</gml:LinearRing>
							</gml:exterior>
						</gml:Polygon>
					</gml:surfaceMember>
				</gml:MultiSurface>
			</tran:lod1MultiSurface>
			<tran:lod2MultiSurface>
				<gml:MultiSurface>
					<gml:surfaceMember xlink:href="#poly_52d9757c-f4b5-480d-8540-02e3f5a2eeca"></gml:surfaceMember>
				</gml:MultiSurface>
			</tran:lod2MultiSurface>
			<tran:lod3MultiSurface>
				<gml:MultiSurface>
					<gml:surfaceMember xlink:href="#poly_3a868b04-d86b-4570-b61a-efe8ef38f5fa"></gml:surfaceMember>
					<gml:surfaceMember xlink:href="#poly_97542112-79ff-42e8-8f43-e274fe4aeb24"></gml:surfaceMember>
					<gml:surfaceMember xlink:href="#poly_6dcb8875-dc3e-4450-a55f-ef3e6a69683a"></gml:surfaceMember>
					<gml:surfaceMember xlink:href="#poly_d4e95347-2d7b-42c0-8317-f3b44cca3ace"></gml:surfaceMember>
					<gml:surfaceMember xlink:href="#poly_5cb10ca1-dc98-4808-ba42-3bdd90d2993d"></gml:surfaceMember>
					<gml:surfaceMember xlink:href="#poly_484b3ab3-b00e-4ad3-8d15-d24aecf9a482"></gml:surfaceMember>
					<gml:surfaceMember xlink:href="#poly_939ecb7b-e781-4838-b0ab-58be46e55604"></gml:surfaceMember>
					<gml:surfaceMember xlink:href="#poly_2a9f3656-b53e-43b3-8360-86dc9f03fcad"></gml:surfaceMember>
				</gml:MultiSurface>
			</tran:lod3MultiSurface>
			<uro:tranDataQualityAttribute>
				<uro:DataQualityAttribute>
					<uro:geometrySrcDescLod1 codeSpace="../../codelists/DataQualityAttribute_geometrySrcDesc.xml">000</uro:geometrySrcDescLod1>
					<uro:geometrySrcDescLod2 codeSpace="../../codelists/DataQualityAttribute_geometrySrcDesc.xml">000</uro:geometrySrcDescLod2>
					<uro:geometrySrcDescLod3 codeSpace="../../codelists/DataQualityAttribute_geometrySrcDesc.xml">000</uro:geometrySrcDescLod3>
					<uro:thematicSrcDesc codeSpace="../../codelists/DataQualityAttribute_thematicSrcDesc.xml">000</uro:thematicSrcDesc>
					<uro:thematicSrcDesc codeSpace="../../codelists/DataQualityAttribute_thematicSrcDesc.xml">023</uro:thematicSrcDesc>
					<uro:lodType codeSpace="../../codelists/Road_lodType.xml">3.0</uro:lodType>
					<uro:publicSurveyDataQualityAttribute>
						<uro:PublicSurveyDataQualityAttribute>
							<uro:srcScaleLod1 codeSpace="../../codelists/PublicSurveyDataQualityAttribute_srcScale.xml">1</uro:srcScaleLod1>
							<uro:srcScaleLod2 codeSpace="../../codelists/PublicSurveyDataQualityAttribute_srcScale.xml">1</uro:srcScaleLod2>
							<uro:srcScaleLod3 codeSpace="../../codelists/PublicSurveyDataQualityAttribute_srcScale.xml">2</uro:srcScaleLod3>
							<uro:publicSurveySrcDescLod1 codeSpace="../../codelists/PublicSurveyDataQualityAttribute_publicSurveySrcDesc.xml">023</uro:publicSurveySrcDescLod1>
							<uro:publicSurveySrcDescLod2 codeSpace="../../codelists/PublicSurveyDataQualityAttribute_publicSurveySrcDesc.xml">023</uro:publicSurveySrcDescLod2>
							<uro:publicSurveySrcDescLod3 codeSpace="../../codelists/PublicSurveyDataQualityAttribute_publicSurveySrcDesc.xml">011</uro:publicSurveySrcDescLod3>
						</uro:PublicSurveyDataQualityAttribute>
					</uro:publicSurveyDataQualityAttribute>
				</uro:DataQualityAttribute>
			</uro:tranDataQualityAttribute>
		</tran:Track>
	</core:cityObjectMember>
	<core:cityObjectMember>
		<tran:Track gml:id="trk_5a9d1556-f670-4d76-ae6b-ad4750173dc3">
			<core:creationDate>2024-03-15</core:creationDate>
			<tran:class codeSpace="../../codelists/TransportationComplex_class.xml">1020</tran:class>
			<tran:function codeSpace="../../codelists/Track_function.xml">2</tran:function>
			<tran:trafficArea>
				<tran:TrafficArea gml:id="tfa_5a5952b7-ac66-485c-b759-cc20cc733d4f">
					<tran:function codeSpace="../../codelists/TrafficArea_function.xml">2000</tran:function>
					<tran:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_84ea446b-6171-4f79-8ffc-bb667188d478">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.701360048989855 139.41238035771556 0 35.70135816201996 139.4128237764615 0 35.70148535253148 139.4128262027319 0 35.70148569780995 139.412796033171 0 35.70148745530939 139.41238289154686 0 35.701360048989855 139.41238035771556 0</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</tran:lod2MultiSurface>
				</tran:TrafficArea>
			</tran:trafficArea>
			<tran:trafficArea>
				<tran:TrafficArea gml:id="tfa_98ccd379-8bd4-42d7-b87b-f4ebfe4f1370">
					<tran:function codeSpace="../../codelists/TrafficArea_function.xml">2000</tran:function>
					<tran:lod3MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_846edfdf-50d4-4a1b-9e92-34f0e9956019">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.70143575650047 139.4128252566435 84.84750323451124 35.7014853525315 139.4128262027319 84.8767963804432 35.70148569780997 139.412796033171 84.87716794189721 35.70143575650047 139.4128252566435 84.84750323451124</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_bca6967b-f0ed-4d27-9a11-0e6471fe62b7">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.701364029525365 139.41238043700983 84.82296297450515 35.70135816201996 139.4128237764615 84.8193162061511 35.70143158554568 139.4123817806489 84.84750323451124 35.701364029525365 139.41238043700983 84.82296297450515</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_75083897-e2ab-4662-9356-613caacb06c2">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.70135816201996 139.4128237764615 84.8193162061511 35.70148569780997 139.412796033171 84.87716794189721 35.70143158554568 139.4123817806489 84.84750323451124 35.70135816201996 139.4128237764615 84.8193162061511</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_f1078429-b3bc-4dfc-8018-236d01283e97">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.70149116461629 139.41238296563375 84.88269243956357 35.70143158554568 139.4123817806489 84.84750323451124 35.70148569780997 139.412796033171 84.87716794189721 35.70149116461629 139.41238296563375 84.88269243956357</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_72ce8e56-006c-427d-8a6a-aeead4ad8475">
									<gml:exterior>
										<gml:LinearRing>
											<gml:posList>35.70143575650047 139.4128252566435 84.84750323451124 35.70148569780997 139.412796033171 84.87716794189721 35.70135816201996 139.4128237764615 84.8193162061511 35.70143575650047 139.4128252566435 84.84750323451124</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</tran:lod3MultiSurface>
				</tran:TrafficArea>
			</tran:trafficArea>
			<tran:lod1MultiSurface>
				<gml:MultiSurface>
					<gml:surfaceMember>
						<gml:Polygon>
							<gml:exterior>
								<gml:LinearRing>
									<gml:posList>35.701360048989855 139.41238035771556 0 35.70135816201996 139.4128237764615 0 35.70148535253148 139.4128262027319 0 35.70148569780995 139.412796033171 0 35.70148745530939 139.41238289154686 0 35.701360048989855 139.41238035771556 0</gml:posList>
								</gml:LinearRing>
							</gml:exterior>
						</gml:Polygon>
					</gml:surfaceMember>
				</gml:MultiSurface>
			</tran:lod1MultiSurface>
			<tran:lod2MultiSurface>
				<gml:MultiSurface>
					<gml:surfaceMember xlink:href="#poly_84ea446b-6171-4f79-8ffc-bb667188d478"></gml:surfaceMember>
				</gml:MultiSurface>
			</tran:lod2MultiSurface>
			<tran:lod3MultiSurface>
				<gml:MultiSurface>
					<gml:surfaceMember xlink:href="#poly_846edfdf-50d4-4a1b-9e92-34f0e9956019"></gml:surfaceMember>
					<gml:surfaceMember xlink:href="#poly_bca6967b-f0ed-4d27-9a11-0e6471fe62b7"></gml:surfaceMember>
					<gml:surfaceMember xlink:href="#poly_75083897-e2ab-4662-9356-613caacb06c2"></gml:surfaceMember>
					<gml:surfaceMember xlink:href="#poly_f1078429-b3bc-4dfc-8018-236d01283e97"></gml:surfaceMember>
					<gml:surfaceMember xlink:href="#poly_72ce8e56-006c-427d-8a6a-aeead4ad8475"></gml:surfaceMember>
				</gml:MultiSurface>
			</tran:lod3MultiSurface>
			<uro:tranDataQualityAttribute>
				<uro:DataQualityAttribute>
					<uro:geometrySrcDescLod1 codeSpace="../../codelists/DataQualityAttribute_geometrySrcDesc.xml">000</uro:geometrySrcDescLod1>
					<uro:geometrySrcDescLod2 codeSpace="../../codelists/DataQualityAttribute_geometrySrcDesc.xml">000</uro:geometrySrcDescLod2>
					<uro:geometrySrcDescLod3 codeSpace="../../codelists/DataQualityAttribute_geometrySrcDesc.xml">000</uro:geometrySrcDescLod3>
					<uro:thematicSrcDesc codeSpace="../../codelists/DataQualityAttribute_thematicSrcDesc.xml">000</uro:thematicSrcDesc>
					<uro:thematicSrcDesc codeSpace="../../codelists/DataQualityAttribute_thematicSrcDesc.xml">023</uro:thematicSrcDesc>
					<uro:lodType codeSpace="../../codelists/Road_lodType.xml">3.0</uro:lodType>
					<uro:publicSurveyDataQualityAttribute>
						<uro:PublicSurveyDataQualityAttribute>
							<uro:srcScaleLod1 codeSpace="../../codelists/PublicSurveyDataQualityAttribute_srcScale.xml">1</uro:srcScaleLod1>
							<uro:srcScaleLod2 codeSpace="../../codelists/PublicSurveyDataQualityAttribute_srcScale.xml">1</uro:srcScaleLod2>
							<uro:srcScaleLod3 codeSpace="../../codelists/PublicSurveyDataQualityAttribute_srcScale.xml">2</uro:srcScaleLod3>
							<uro:publicSurveySrcDescLod1 codeSpace="../../codelists/PublicSurveyDataQualityAttribute_publicSurveySrcDesc.xml">023</uro:publicSurveySrcDescLod1>
							<uro:publicSurveySrcDescLod2 codeSpace="../../codelists/PublicSurveyDataQualityAttribute_publicSurveySrcDesc.xml">023</uro:publicSurveySrcDescLod2>
							<uro:publicSurveySrcDescLod3 codeSpace="../../codelists/PublicSurveyDataQualityAttribute_publicSurveySrcDesc.xml">011</uro:publicSurveySrcDescLod3>
						</uro:PublicSurveyDataQualityAttribute>
					</uro:publicSurveyDataQualityAttribute>
				</uro:DataQualityAttribute>
			</uro:tranDataQualityAttribute>
		</tran:Track>
	</core:cityObjectMember>

</core:CityModel>
