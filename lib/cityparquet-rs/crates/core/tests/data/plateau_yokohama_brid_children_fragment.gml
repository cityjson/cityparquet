<?xml version="1.0" encoding="UTF-8"?>
<!--
  Test fragment from Japan's PLATEAU 3D City Models (Kanagawa, Yokohama-shi;
  14100_yokohama-shi_city_2024_citygml_2_op), CityGML 2.0 module `brid`,
  tile 53391459_brid_6697_op.gml. Licence: CC BY 4.0 (Project PLATEAU, MLIT
  Japan).

  Root CityModel + gml:Envelope + ONE cityObjectMember, verbatim; the
  document-level app:appearanceMember is dropped.

  A `brid:Bridge` carrying THREE `brid:BridgeConstructionElement` children
  under `brid:outerBridgeConstruction` — the bridge counterpart of a
  Building's `bldg:outerBuildingInstallation`. CityGML 2.0 spells the type
  `BridgeConstructionElement`; CityJSON (and CityGML 3.0) spell it
  `BridgeConstructiveElement`, so it is one of the remapped names.
-->
<core:CityModel xmlns:brid="http://www.opengis.net/citygml/bridge/2.0" xmlns:tran="http://www.opengis.net/citygml/transportation/2.0" xmlns:frn="http://www.opengis.net/citygml/cityfurniture/2.0" xmlns:wtr="http://www.opengis.net/citygml/waterbody/2.0" xmlns:sch="http://www.ascc.net/xml/schematron" xmlns:veg="http://www.opengis.net/citygml/vegetation/2.0" xmlns:xlink="http://www.w3.org/1999/xlink" xmlns:tun="http://www.opengis.net/citygml/tunnel/2.0" xmlns:tex="http://www.opengis.net/citygml/texturedsurface/2.0" xmlns:gml="http://www.opengis.net/gml" xmlns:app="http://www.opengis.net/citygml/appearance/2.0" xmlns:gen="http://www.opengis.net/citygml/generics/2.0" xmlns:dem="http://www.opengis.net/citygml/relief/2.0" xmlns:luse="http://www.opengis.net/citygml/landuse/2.0" xmlns:uro="https://www.geospatial.jp/iur/uro/3.1" xmlns:xAL="urn:oasis:names:tc:ciq:xsdschema:xAL:2.0" xmlns:bldg="http://www.opengis.net/citygml/building/2.0" xmlns:smil20="http://www.w3.org/2001/SMIL20/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:smil20lang="http://www.w3.org/2001/SMIL20/Language" xmlns:pbase="http://www.opengis.net/citygml/profiles/base/2.0" xmlns:core="http://www.opengis.net/citygml/2.0" xmlns:grp="http://www.opengis.net/citygml/cityobjectgroup/2.0" xsi:schemaLocation="https://www.geospatial.jp/iur/uro/3.1 ../../schemas/iur/uro/3.1/urbanObject.xsd http://www.opengis.net/citygml/2.0 http://schemas.opengis.net/citygml/2.0/cityGMLBase.xsd http://www.opengis.net/citygml/landuse/2.0 http://schemas.opengis.net/citygml/landuse/2.0/landUse.xsd http://www.opengis.net/citygml/building/2.0 http://schemas.opengis.net/citygml/building/2.0/building.xsd http://www.opengis.net/citygml/transportation/2.0 http://schemas.opengis.net/citygml/transportation/2.0/transportation.xsd http://www.opengis.net/citygml/generics/2.0 http://schemas.opengis.net/citygml/generics/2.0/generics.xsd http://www.opengis.net/citygml/cityobjectgroup/2.0 http://schemas.opengis.net/citygml/cityobjectgroup/2.0/cityObjectGroup.xsd http://www.opengis.net/gml http://schemas.opengis.net/gml/3.1.1/base/gml.xsd http://www.opengis.net/citygml/appearance/2.0 http://schemas.opengis.net/citygml/appearance/2.0/appearance.xsd">
	<gml:boundedBy>
		<gml:Envelope srsName="http://www.opengis.net/def/crs/EPSG/0/6697" srsDimension="3">
			<gml:lowerCorner>35.458981040061694 139.61388822436427 -5.40387417</gml:lowerCorner>
			<gml:upperCorner>35.46652021723128 139.6250504600989 24.44694595</gml:upperCorner>
		</gml:Envelope>
	</gml:boundedBy>
	<core:cityObjectMember>
		<brid:Bridge gml:id="brid_2bd73adb-5eba-4701-b35e-ffb2681312ac">
			<core:creationDate>2024-03-22</core:creationDate>
			<brid:class codeSpace="../../codelists/Bridge_class.xml">01</brid:class>
			<brid:function codeSpace="../../codelists/Bridge_function.xml">07</brid:function>
			<brid:yearOfConstruction>0001</brid:yearOfConstruction>
			<brid:lod2Solid>
				<gml:Solid>
					<gml:exterior>
						<gml:CompositeSurface>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p31_0"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p31_1"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p31_2"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p31_3"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p31_4"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p30_0"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p30_1"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p30_2"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p30_3"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p30_4"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p30_5"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p30_6"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p30_7"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p30_8"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p26_0"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p26_1"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p26_2"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p26_3"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p26_4"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p26_5"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p26_6"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p26_7"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p26_8"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p26_9"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p26_10"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p26_11"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p26_12"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p26_13"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p26_14"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p29_0"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p29_1"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p29_2"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p29_3"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p29_4"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p28_0"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p28_1"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p28_2"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p28_3"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p28_4"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p25_0"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p25_1"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p25_2"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p25_3"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p24_0"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p24_1"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p24_2"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p24_3"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_p24_4"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_b_0"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0013_b_1"/>
						</gml:CompositeSurface>
					</gml:exterior>
				</gml:Solid>
			</brid:lod2Solid>
			<brid:outerBridgeConstruction>
				<brid:BridgeConstructionElement gml:id="brid_2280e1b7-ecb9-4838-aa5f-6379912a3e3c">
					<brid:lod2Geometry>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p32772_0">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p32772_0">
											<gml:posList>35.46510874129916 139.6233674709955 1.44031332 35.465101585527854 139.62337870869243 1.43939755 35.465096562004014 139.62337392977167 1.43939503 35.465103717774596 139.62336269207472 1.4403108 35.46510874129916 139.6233674709955 1.44031332</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p32_0">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p32_0">
											<gml:posList>35.465103717774596 139.62336269207472 1.4403108 35.46510370597156 139.62336271042363 4.43031005 35.465108729493764 139.62336748934226 4.43031257 35.46510874129916 139.6233674709955 1.44031332 35.465103717774596 139.62336269207472 1.4403108</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p32_1">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p32_1">
											<gml:posList>35.46510874129916 139.6233674709955 1.44031332 35.465108729493764 139.62336748934226 4.43031257 35.465101573725796 139.62337872703398 4.4293968 35.465101585527854 139.62337870869243 1.43939755 35.46510874129916 139.6233674709955 1.44031332</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p32_2">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p32_2">
											<gml:posList>35.465101585527854 139.62337870869243 1.43939755 35.465101573725796 139.62337872703398 4.4293968 35.46509655020438 139.62337394811541 4.42939428 35.465096562004014 139.62337392977167 1.43939503 35.465101585527854 139.62337870869243 1.43939755</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p32_3">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p32_3">
											<gml:posList>35.465096562004014 139.62337392977167 1.43939503 35.46509655020438 139.62337394811541 4.42939428 35.46510370597156 139.62336271042363 4.43031005 35.465103717774596 139.62336269207472 1.4403108 35.465096562004014 139.62337392977167 1.43939503</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2Geometry>
				</brid:BridgeConstructionElement>
			</brid:outerBridgeConstruction>
			<brid:outerBridgeConstruction>
				<brid:BridgeConstructionElement gml:id="brid_6c74b3ea-e50b-4716-9d1d-f4cfbafe622f">
					<brid:lod2Geometry>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p32771_0">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p32771_0">
											<gml:posList>35.465260460276134 139.623824550171 1.42473702 35.46522449459309 139.62382560361578 1.42292704 35.465224246200115 139.62381010359374 1.42368949 35.46526019920577 139.62380900891836 1.42550092 35.465260460276134 139.623824550171 1.42473702</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p33_0">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p33_0">
											<gml:posList>35.46526019920577 139.62380900891836 1.42550092 35.465260177200044 139.62380904252882 6.96549954 35.46526043827018 139.62382458376788 6.96473564 35.465260460276134 139.623824550171 1.42473702 35.46526019920577 139.62380900891836 1.42550092</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p33_1">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p33_1">
											<gml:posList>35.465260460276134 139.623824550171 1.42473702 35.46526043827018 139.62382458376788 6.96473564 35.465224472618424 139.62382563721178 6.96292566 35.46522449459309 139.62382560361578 1.42292704 35.465260460276134 139.623824550171 1.42473702</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p33_2">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p33_2">
											<gml:posList>35.46522449459309 139.62382560361578 1.42292704 35.465224472618424 139.62382563721178 6.96292566 35.46522422422565 139.6238101372032 6.96368812 35.465224246200115 139.62381010359374 1.42368949 35.46522449459309 139.62382560361578 1.42292704</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p33_3">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p33_3">
											<gml:posList>35.465224246200115 139.62381010359374 1.42368949 35.46522422422565 139.6238101372032 6.96368812 35.465260177200044 139.62380904252882 6.96549954 35.46526019920577 139.62380900891836 1.42550092 35.465224246200115 139.62381010359374 1.42368949</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2Geometry>
				</brid:BridgeConstructionElement>
			</brid:outerBridgeConstruction>
			<brid:outerBridgeConstruction>
				<brid:BridgeConstructionElement gml:id="brid_bf1663f7-ba9b-4596-a5a9-ea9b07ad1528">
					<brid:lod2Geometry>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p32770_0">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p32770_0">
											<gml:posList>35.46525583764215 139.62357434345145 1.43705284 35.46521988461976 139.62357543824217 1.43524138 35.46521956798288 139.62355992159016 1.43600635 35.465255521006455 139.62355882679168 1.4378178 35.46525583764215 139.62357434345145 1.43705284</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p27_0">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p27_0">
											<gml:posList>35.465255521006455 139.62355882679168 1.4378178 35.46525549900486 139.62355886061937 6.97781642 35.46525581564035 139.62357437726558 6.97705145 35.46525583764215 139.62357434345145 1.43705284 35.465255521006455 139.62355882679168 1.4378178</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p27_1">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p27_1">
											<gml:posList>35.46525583764215 139.62357434345145 1.43705284 35.46525581564035 139.62357437726558 6.97705145 35.46521986264924 139.6235754720553 6.97524 35.46521988461976 139.62357543824217 1.43524138 35.46525583764215 139.62357434345145 1.43705284</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p27_2">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p27_2">
											<gml:posList>35.46521988461976 139.62357543824217 1.43524138 35.46521986264924 139.6235754720553 6.97524 35.46521954601266 139.62355995541677 6.97600496 35.46521956798288 139.62355992159016 1.43600635 35.46521988461976 139.62357543824217 1.43524138</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p27_3">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p27_3">
											<gml:posList>35.46521956798288 139.62355992159016 1.43600635 35.46521954601266 139.62355995541677 6.97600496 35.46525549900486 139.62355886061937 6.97781642 35.465255521006455 139.62355882679168 1.4378178 35.46521956798288 139.62355992159016 1.43600635</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2Geometry>
				</brid:BridgeConstructionElement>
			</brid:outerBridgeConstruction>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0013_p31_1">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p31_1">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p31_1">
											<gml:posList>35.46501737766975 139.6233059211952 1.548995 35.46501737373582 139.62330592734085 2.54899475 35.46503066621268 139.62328505228578 2.55069588 35.46503067014867 139.6232850461369 1.55069613 35.46501737766975 139.6233059211952 1.548995</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0013_p31_3">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p31_3">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p31_3">
											<gml:posList>35.465104216307175 139.62335626443485 4.43066079 35.46503067014867 139.6232850461369 1.55069613 35.46503066621268 139.62328505228578 2.55069588 35.46510421235952 139.62335627057269 5.43066054 35.465104216307175 139.62335626443485 4.43066079</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0013_p31_4">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p31_4">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p31_4">
											<gml:posList>35.46501737766975 139.6233059211952 1.548995 35.46509093912325 139.6233771154351 4.42896164 35.465090935177756 139.62337712156963 5.42896139 35.46501737373582 139.62330592734085 2.54899475 35.46501737766975 139.6233059211952 1.548995</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0013_p30_6">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p30_6">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p30_6">
											<gml:posList>35.46510421235952 139.62335627057269 5.43066054 35.465114887971176 139.62336642637084 5.43066592 35.46518929119166 139.62343713074108 7.9807177 35.46518929515268 139.62343712461595 6.98071795 35.46511489192044 139.62336642023462 4.43066617 35.465104216307175 139.62335626443485 4.43066079 35.46510421235952 139.62335627057269 5.43066054</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0013_p30_7">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p30_7">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p30_7">
											<gml:posList>35.465090935177756 139.62337712156963 5.42896139 35.46509093912325 139.6233771154351 4.42896164 35.465101614733534 139.62338727123455 4.42896702 35.46510161078632 139.62338727736747 5.42896677 35.465090935177756 139.62337712156963 5.42896139</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0013_p30_8">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p30_8">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p30_8">
											<gml:posList>35.46510161078632 139.62338727736747 5.42896677 35.465101614733534 139.62338727123455 4.42896702 35.465177334801794 139.62345525503073 6.97922019 35.465177330842664 139.62345526115297 7.97921994 35.46510161078632 139.62338727736747 5.42896677</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0013_p26_2">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p26_2">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p26_2">
											<gml:posList>35.46527875846623 139.62382995740785 6.96536329 35.465278754491194 139.6238299634715 7.96536304 35.46520362016498 139.6238312336518 7.96162857 35.46520362412819 139.62383122758837 6.96162882 35.46527875846623 139.62382995740785 6.96536329</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0013_p26_3">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p26_3">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p26_3">
											<gml:posList>35.46520362412819 139.62383122758837 6.96162882 35.46520362016498 139.6238312336518 7.96162857 35.46520342831721 139.6235594356192 7.97524479 35.46520343228043 139.62355942951325 6.97524504 35.46520362412819 139.62383122758837 6.96162882</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0013_p26_4">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p26_4">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p26_4">
											<gml:posList>35.465069618465286 139.6234368471739 6.97491055 35.46520343228043 139.62355942951325 6.97524504 35.46520342831721 139.6235594356192 7.97524479 35.46506961452311 139.62343685329907 7.9749103 35.465069618465286 139.6234368471739 6.97491055</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0013_p26_5">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p26_5">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p26_5">
											<gml:posList>35.46505537264127 139.62335371804465 5.34841861 35.46509738857905 139.6233916016342 7.97854378 35.46517388564489 139.6234604836455 7.97878863 35.46517388960347 139.62346047752402 6.97878888 35.46509739252555 139.62339159550197 6.97854403 35.46505537658116 139.6233537119065 4.34841886 35.46505537264127 139.62335371804465 5.34841861</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0013_p26_6">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p26_6">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p26_6">
											<gml:posList>35.46517388960347 139.62346047752402 6.97878888 35.46517388564489 139.6234604836455 7.97878863 35.465177330842664 139.62345526115297 7.97921994 35.465177334801794 139.62345525503073 6.97922019 35.46517388960347 139.62346047752402 6.97878888</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0013_p26_7">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p26_7">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p26_7">
											<gml:posList>35.465200864678955 139.62343726723765 6.981275 35.46518929515268 139.62343712461595 6.98071795 35.46518929119166 139.62343713074108 7.9807177 35.465200860716145 139.62343727336267 7.98127474 35.465200864678955 139.62343726723765 6.981275</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0013_p26_8">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p26_8">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p26_8">
											<gml:posList>35.465200864678955 139.62343726723765 6.981275 35.465200860716145 139.62343727336267 7.98127474 35.46520052805775 139.62339850544546 7.98321588 35.46520053202047 139.62339849931428 6.98321614 35.465200864678955 139.62343726723765 6.981275</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0013_p26_9">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p26_9">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p26_9">
											<gml:posList>35.46520052805775 139.62339850544546 7.98321588 35.46519664224411 139.62339524309832 7.98319115 35.46519664620628 139.6233952369667 6.9831914 35.46520053202047 139.62339849931428 6.98321614 35.46520052805775 139.62339850544546 7.98321588</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0013_p26_10">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p26_10">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p26_10">
											<gml:posList>35.4652357070248 139.62339211917808 6.9852555 35.46521200406317 139.6233712016186 6.98515528 35.46521200009865 139.623371207754 7.98515503 35.4652357030565 139.6233921253103 7.98525524 35.4652357070248 139.62339211917808 6.9852555</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0013_p26_11">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p26_11">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p26_11">
											<gml:posList>35.4652357070248 139.62339211917808 6.9852555 35.4652357030565 139.6233921253103 7.98525524 35.465246961431724 139.6233924169098 7.98579059 35.46524696540175 139.6233924107776 6.98579084 35.4652357070248 139.62339211917808 6.9852555</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0013_p26_12">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p26_12">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p26_12">
											<gml:posList>35.46524696540175 139.6233924107776 6.98579084 35.465246961431724 139.6233924169098 7.98579059 35.46526816099423 139.6234002572418 7.98643098 35.465268164967576 139.6234002511109 6.98643123 35.46524696540175 139.6233924107776 6.98579084</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0013_p26_13">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p26_13">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p26_13">
											<gml:posList>35.465268164967576 139.6234002511109 6.98643123 35.46526816099423 139.6234002572418 7.98643098 35.465268601089534 139.6235484444848 7.97898143 35.46526860506303 139.62354843837718 6.97898168 35.465268164967576 139.6234002511109 6.98643123</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0013_p26_14">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p26_14">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p26_14">
											<gml:posList>35.46526860506303 139.62354843837718 6.97898168 35.465268601089534 139.6235484444848 7.97898143 35.465278754491194 139.6238299634715 7.96536304 35.46527875846623 139.62382995740785 6.96536329 35.46526860506303 139.62354843837718 6.97898168</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0013_p29_1">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p29_1">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p29_1">
											<gml:posList>35.465219497568604 139.6233788390397 9.32513559 35.46521949360291 139.62337884517387 10.32513534 35.465203343458434 139.62340513348457 10.32301903 35.46520334742161 139.62340512735446 9.32301928 35.465219497568604 139.6233788390397 9.32513559</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0013_p29_3">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p29_3">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p29_3">
											<gml:posList>35.465203343458434 139.62340513348457 10.32301903 35.4651951358891 139.62339760041093 10.32299919 35.465195139851005 139.6233975942795 9.32299944 35.46520334742161 139.62340512735446 9.32301928 35.465203343458434 139.62340513348457 10.32301903</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0013_p29_4">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p29_4">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p29_4">
											<gml:posList>35.465219497568604 139.6233788390397 9.32513559 35.465211698005035 139.62337168044405 9.32511673 35.46521169404053 139.62337168657942 10.32511648 35.46521949360291 139.62337884517387 10.32513534 35.465219497568604 139.6233788390397 9.32513559</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0013_p28_0">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p28_0">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p28_0">
											<gml:posList>35.46521255206208 139.62337034382753 9.32522597 35.465212544033704 139.62337035625202 11.35022546 35.465193701918075 139.6233998445319 11.34781615 35.46519370994042 139.62339983211677 9.32281666 35.465193719210724 139.62339981777035 6.98281725 35.4651937411584 139.62339978380476 1.44281864 35.46521258330341 139.62337029547905 1.44522796 35.4652125573747 139.62337033560587 7.98522631 35.46521255206208 139.62337034382753 9.32522597</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
									<gml:interior>
										<gml:LinearRing gml:id="line_YHHW0013_p28_0_hole0">
											<gml:posList>35.46521169404053 139.62337168657942 10.32511648 35.465211698005035 139.62337168044405 9.32511673 35.465195139851005 139.6233975942795 9.32299944 35.4651951358891 139.62339760041093 10.32299919 35.46521169404053 139.62337168657942 10.32511648</gml:posList>
										</gml:LinearRing>
									</gml:interior>
									<gml:interior>
										<gml:LinearRing gml:id="line_YHHW0013_p28_0_hole1">
											<gml:posList>35.46519664224411 139.62339524309832 7.98319115 35.46521200009865 139.623371207754 7.98515503 35.46521200406317 139.6233712016186 6.98515528 35.46519664620628 139.6233952369667 6.9831914 35.46519664224411 139.62339524309832 7.98319115</gml:posList>
										</gml:LinearRing>
									</gml:interior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0013_p28_2">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p28_2">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p28_2">
											<gml:posList>35.46519370994042 139.62339983211677 9.32281666 35.465193701918075 139.6233998445319 11.34781615 35.465167994462895 139.62337530410989 11.34780311 35.465168033663204 139.6233752433445 1.4428056 35.4651937411584 139.62339978380476 1.44281864 35.465193719210724 139.62339981777035 6.98281725 35.46519370994042 139.62339983211677 9.32281666</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0013_p28_3">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p28_3">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p28_3">
											<gml:posList>35.465168033663204 139.6233752433445 1.4428056 35.465167994462895 139.62337530410989 11.34780311 35.46518683656846 139.6233458158288 11.35021241 35.46518687579806 139.62334575501768 1.44521491 35.465168033663204 139.6233752433445 1.4428056</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0013_p28_4">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p28_4">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p28_4">
											<gml:posList>35.46518687579806 139.62334575501768 1.44521491 35.46518683656846 139.6233458158288 11.35021241 35.465212544033704 139.62337035625202 11.35022546 35.46521255206208 139.62337034382753 9.32522597 35.4652125573747 139.62337033560587 7.98522631 35.46521258330341 139.62337029547905 1.44522796 35.46518687579806 139.62334575501768 1.44521491</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0013_p25_0">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p25_0">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p25_0">
											<gml:posList>35.4650553880463 139.62335369404434 1.43841959 35.46504842462212 139.62334731950267 1.43840431 35.465016346411986 139.6233180078373 1.43833313 35.465016339350925 139.62331801886538 3.23333268 35.46504840922133 139.62334734350685 5.34840333 35.46505537264127 139.62335371804465 5.34841861 35.46505537658116 139.6233537119065 4.34841886 35.4650553880463 139.62335369404434 1.43841959</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0013_p25_2">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p25_2">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p25_2">
											<gml:posList>35.46502758561645 139.62339906608207 5.34477968 35.46502062220058 139.62339269154458 5.34476441 35.46498855233235 139.62336336693545 3.22969376 35.46498855938559 139.62336335592008 1.43469421 35.46502063758431 139.62339266756814 1.43476539 35.465027601004415 139.62339904210947 1.43478066 35.465027589551944 139.623399059951 4.34477993 35.46502758561645 139.62339906608207 5.34477968</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0013_p25_3">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p25_3">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p25_3">
											<gml:posList>35.46498855233235 139.62336336693545 3.22969376 35.465016339350925 139.62331801886538 3.23333268 35.465016346411986 139.6233180078373 1.43833313 35.46498855938559 139.62336335592008 1.43469421 35.46498855233235 139.62336336693545 3.22969376</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0013_p24_2">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p24_2">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p24_2">
											<gml:posList>35.465027601004415 139.62339904210947 1.43478066 35.4650553880463 139.62335369404434 1.43841959 35.46505537658116 139.6233537119065 4.34841886 35.465027589551944 139.623399059951 4.34477993 35.465027601004415 139.62339904210947 1.43478066</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0013_p24_4">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p24_4">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p24_4">
											<gml:posList>35.46502758561645 139.62339906608207 5.34477968 35.465027589551944 139.623399059951 4.34477993 35.465069618465286 139.6234368471739 6.97491055 35.46506961452311 139.62343685329907 7.9749103 35.46502758561645 139.62339906608207 5.34477968</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:RoofSurface gml:id="roof_YHHW0013_p29_0">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p29_0">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p29_0">
											<gml:posList>35.46521949360291 139.62337884517387 10.32513534 35.46521169404053 139.62337168657942 10.32511648 35.4651951358891 139.62339760041093 10.32299919 35.465203343458434 139.62340513348457 10.32301903 35.46521949360291 139.62337884517387 10.32513534</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:RoofSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:RoofSurface gml:id="roof_YHHW0013_p28_1">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p28_1">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p28_1">
											<gml:posList>35.46518683656846 139.6233458158288 11.35021241 35.465167994462895 139.62337530410989 11.34780311 35.465193701918075 139.6233998445319 11.34781615 35.465212544033704 139.62337035625202 11.35022546 35.46518683656846 139.6233458158288 11.35021241</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:RoofSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:OuterCeilingSurface gml:id="ceil_YHHW0013_p31_2">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p31_2">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p31_2">
											<gml:posList>35.465104216307175 139.62335626443485 4.43066079 35.46509093912325 139.6233771154351 4.42896164 35.46501737766975 139.6233059211952 1.548995 35.46503067014867 139.6232850461369 1.55069613 35.465104216307175 139.62335626443485 4.43066079</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:OuterCeilingSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:OuterCeilingSurface gml:id="ceil_YHHW0013_p30_3">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p30_3">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p30_3">
											<gml:posList>35.46511489192044 139.62336642023462 4.43066617 35.465101614733534 139.62338727123455 4.42896702 35.46509093912325 139.6233771154351 4.42896164 35.465104216307175 139.62335626443485 4.43066079 35.46511489192044 139.62336642023462 4.43066617</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:OuterCeilingSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:OuterCeilingSurface gml:id="ceil_YHHW0013_p30_4">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p30_4">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p30_4">
											<gml:posList>35.46511489192044 139.62336642023462 4.43066617 35.465177334801794 139.62345525503073 6.97922019 35.465101614733534 139.62338727123455 4.42896702 35.46511489192044 139.62336642023462 4.43066617</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:OuterCeilingSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:OuterCeilingSurface gml:id="ceil_YHHW0013_p30_5">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p30_5">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p30_5">
											<gml:posList>35.465177334801794 139.62345525503073 6.97922019 35.46511489192044 139.62336642023462 4.43066617 35.46518929515268 139.62343712461595 6.98071795 35.465177334801794 139.62345525503073 6.97922019</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:OuterCeilingSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:OuterCeilingSurface gml:id="ceil_YHHW0013_p26_1">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p26_1">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p26_1">
											<gml:posList>35.46520053202047 139.62339849931428 6.98321614 35.46519664620628 139.6233952369667 6.9831914 35.46521200406317 139.6233712016186 6.98515528 35.4652357070248 139.62339211917808 6.9852555 35.46524696540175 139.6233924107776 6.98579084 35.465268164967576 139.6234002511109 6.98643123 35.46526860506303 139.62354843837718 6.97898168 35.46527875846623 139.62382995740785 6.96536329 35.46520362412819 139.62383122758837 6.96162882 35.46520343228043 139.62355942951325 6.97524504 35.465069618465286 139.6234368471739 6.97491055 35.46509739252555 139.62339159550197 6.97854403 35.46517388960347 139.62346047752402 6.97878888 35.465177334801794 139.62345525503073 6.97922019 35.46518929515268 139.62343712461595 6.98071795 35.465200864678955 139.62343726723765 6.981275 35.46520053202047 139.62339849931428 6.98321614</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:OuterCeilingSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:OuterCeilingSurface gml:id="ceil_YHHW0013_p29_2">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p29_2">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p29_2">
											<gml:posList>35.46520334742161 139.62340512735446 9.32301928 35.465195139851005 139.6233975942795 9.32299944 35.465211698005035 139.62337168044405 9.32511673 35.465219497568604 139.6233788390397 9.32513559 35.46520334742161 139.62340512735446 9.32301928</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:OuterCeilingSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:OuterCeilingSurface gml:id="ceil_YHHW0013_p24_3">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p24_3">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p24_3">
											<gml:posList>35.465027589551944 139.623399059951 4.34477993 35.46505537658116 139.6233537119065 4.34841886 35.46509739252555 139.62339159550197 6.97854403 35.465069618465286 139.6234368471739 6.97491055 35.465027589551944 139.623399059951 4.34477993</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:OuterCeilingSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:OuterFloorSurface gml:id="floor_YHHW0013_p31_0">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p31_0">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p31_0">
											<gml:posList>35.46501737373582 139.62330592734085 2.54899475 35.465090935177756 139.62337712156963 5.42896139 35.46510421235952 139.62335627057269 5.43066054 35.46503066621268 139.62328505228578 2.55069588 35.46501737373582 139.62330592734085 2.54899475</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:OuterFloorSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:OuterFloorSurface gml:id="floor_YHHW0013_p30_0">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p30_0">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p30_0">
											<gml:posList>35.46510161078632 139.62338727736747 5.42896677 35.465114887971176 139.62336642637084 5.43066592 35.46510421235952 139.62335627057269 5.43066054 35.465090935177756 139.62337712156963 5.42896139 35.46510161078632 139.62338727736747 5.42896677</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:OuterFloorSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:OuterFloorSurface gml:id="floor_YHHW0013_p30_1">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p30_1">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p30_1">
											<gml:posList>35.465114887971176 139.62336642637084 5.43066592 35.46510161078632 139.62338727736747 5.42896677 35.465177330842664 139.62345526115297 7.97921994 35.465114887971176 139.62336642637084 5.43066592</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:OuterFloorSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:OuterFloorSurface gml:id="floor_YHHW0013_p30_2">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p30_2">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p30_2">
											<gml:posList>35.465177330842664 139.62345526115297 7.97921994 35.46518929119166 139.62343713074108 7.9807177 35.465114887971176 139.62336642637084 5.43066592 35.465177330842664 139.62345526115297 7.97921994</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:OuterFloorSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:OuterFloorSurface gml:id="floor_YHHW0013_p26_0">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p26_0">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p26_0">
											<gml:posList>35.46520362016498 139.6238312336518 7.96162857 35.465278754491194 139.6238299634715 7.96536304 35.465268601089534 139.6235484444848 7.97898143 35.46526816099423 139.6234002572418 7.98643098 35.465246961431724 139.6233924169098 7.98579059 35.4652357030565 139.6233921253103 7.98525524 35.46521200009865 139.623371207754 7.98515503 35.46519664224411 139.62339524309832 7.98319115 35.46520052805775 139.62339850544546 7.98321588 35.465200860716145 139.62343727336267 7.98127474 35.46518929119166 139.62343713074108 7.9807177 35.465177330842664 139.62345526115297 7.97921994 35.46517388564489 139.6234604836455 7.97878863 35.46509738857905 139.6233916016342 7.97854378 35.46506961452311 139.62343685329907 7.9749103 35.46520342831721 139.6235594356192 7.97524479 35.46520362016498 139.6238312336518 7.96162857</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:OuterFloorSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:OuterFloorSurface gml:id="floor_YHHW0013_p25_1">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p25_1">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p25_1">
											<gml:posList>35.46504840922133 139.62334734350685 5.34840333 35.465016339350925 139.62331801886538 3.23333268 35.46498855233235 139.62336336693545 3.22969376 35.46502062220058 139.62339269154458 5.34476441 35.46504840922133 139.62334734350685 5.34840333</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:OuterFloorSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:OuterFloorSurface gml:id="floor_YHHW0013_p24_0">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p24_0">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p24_0">
											<gml:posList>35.46502758561645 139.62339906608207 5.34477968 35.46506961452311 139.62343685329907 7.9749103 35.46509738857905 139.6233916016342 7.97854378 35.46505537264127 139.62335371804465 5.34841861 35.46502758561645 139.62339906608207 5.34477968</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:OuterFloorSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:OuterFloorSurface gml:id="floor_YHHW0013_p24_1">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_p24_1">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_p24_1">
											<gml:posList>35.46505537264127 139.62335371804465 5.34841861 35.46504840922133 139.62334734350685 5.34840333 35.46502062220058 139.62339269154458 5.34476441 35.46502758561645 139.62339906608207 5.34477968 35.46505537264127 139.62335371804465 5.34841861</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:OuterFloorSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:GroundSurface gml:id="gnd_YHHW0013_b_0">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_b_0">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_b_0">
											<gml:posList>35.46498855938559 139.62336335592008 1.43469421 35.465016346411986 139.6233180078373 1.43833313 35.46504842462212 139.62334731950267 1.43840431 35.4650553880463 139.62335369404434 1.43841959 35.465027601004415 139.62339904210947 1.43478066 35.46502063758431 139.62339266756814 1.43476539 35.46498855938559 139.62336335592008 1.43469421</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:GroundSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:GroundSurface gml:id="gnd_YHHW0013_b_1">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0013_b_1">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0013_b_1">
											<gml:posList>35.4651937411584 139.62339978380476 1.44281864 35.465168033663204 139.6233752433445 1.4428056 35.46518687579806 139.62334575501768 1.44521491 35.46521258330341 139.62337029547905 1.44522796 35.4651937411584 139.62339978380476 1.44281864</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:GroundSurface>
			</brid:boundedBy>
			<uro:bridDataQualityAttribute>
				<uro:DataQualityAttribute>
					<uro:geometrySrcDescLod1 codeSpace="../../codelists/DataQualityAttribute_geometrySrcDesc.xml">999</uro:geometrySrcDescLod1>
					<uro:geometrySrcDescLod2 codeSpace="../../codelists/DataQualityAttribute_geometrySrcDesc.xml">000</uro:geometrySrcDescLod2>
					<uro:geometrySrcDescLod3 codeSpace="../../codelists/DataQualityAttribute_geometrySrcDesc.xml">999</uro:geometrySrcDescLod3>
					<uro:thematicSrcDesc codeSpace="../../codelists/DataQualityAttribute_thematicSrcDesc.xml">000</uro:thematicSrcDesc>
					<uro:appearanceSrcDescLod2 codeSpace="../../codelists/DataQualityAttribute_appearanceSrcDesc.xml">1</uro:appearanceSrcDescLod2>
					<uro:appearanceSrcDescLod3 codeSpace="../../codelists/DataQualityAttribute_appearanceSrcDesc.xml">99</uro:appearanceSrcDescLod3>
					<uro:lodType codeSpace="../../codelists/Bridge_lodType.xml">2.1</uro:lodType>
					<uro:publicSurveyDataQualityAttribute>
						<uro:PublicSurveyDataQualityAttribute>
							<uro:srcScaleLod2 codeSpace="../../codelists/PublicSurveyDataQualityAttribute_srcScale.xml">2</uro:srcScaleLod2>
							<uro:publicSurveySrcDescLod2 codeSpace="../../codelists/PublicSurveyDataQualityAttribute_publicSurveySrcDesc.xml">003</uro:publicSurveySrcDescLod2>
						</uro:PublicSurveyDataQualityAttribute>
					</uro:publicSurveyDataQualityAttribute>
				</uro:DataQualityAttribute>
			</uro:bridDataQualityAttribute>
		</brid:Bridge>
	</core:cityObjectMember>
</core:CityModel>
