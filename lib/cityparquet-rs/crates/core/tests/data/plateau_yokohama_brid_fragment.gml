<?xml version="1.0" encoding="UTF-8"?>
<!--
  Test fragment from Japan's PLATEAU 3D City Models (Kanagawa, Yokohama-shi;
  14100_yokohama-shi_city_2024_citygml_2_op), CityGML 2.0 module `brid`,
  tile 53391459_brid_6697_op.gml. Licence: CC BY 4.0 (Project PLATEAU, MLIT
  Japan).

  Root CityModel + gml:Envelope + ONE cityObjectMember, verbatim; the
  document-level app:appearanceMember is dropped (megabytes of texture
  coordinates for surfaces this fragment does not keep).

  The member is a self-contained `brid:Bridge` in the shape CityGML 2.0
  prescribes and this reader could not read: its twenty-one boundary polygons
  are defined inside `brid:boundedBy` semantic surfaces
  (`brid:OuterFloorSurface` and friends, each holding a `brid:lod2MultiSurface`),
  and its `brid:lod2Solid` composes exactly those twenty-one by
  `gml:surfaceMember xlink:href`. Every reference resolves within the member.
-->
<core:CityModel xmlns:brid="http://www.opengis.net/citygml/bridge/2.0" xmlns:tran="http://www.opengis.net/citygml/transportation/2.0" xmlns:frn="http://www.opengis.net/citygml/cityfurniture/2.0" xmlns:wtr="http://www.opengis.net/citygml/waterbody/2.0" xmlns:sch="http://www.ascc.net/xml/schematron" xmlns:veg="http://www.opengis.net/citygml/vegetation/2.0" xmlns:xlink="http://www.w3.org/1999/xlink" xmlns:tun="http://www.opengis.net/citygml/tunnel/2.0" xmlns:tex="http://www.opengis.net/citygml/texturedsurface/2.0" xmlns:gml="http://www.opengis.net/gml" xmlns:app="http://www.opengis.net/citygml/appearance/2.0" xmlns:gen="http://www.opengis.net/citygml/generics/2.0" xmlns:dem="http://www.opengis.net/citygml/relief/2.0" xmlns:luse="http://www.opengis.net/citygml/landuse/2.0" xmlns:uro="https://www.geospatial.jp/iur/uro/3.1" xmlns:xAL="urn:oasis:names:tc:ciq:xsdschema:xAL:2.0" xmlns:bldg="http://www.opengis.net/citygml/building/2.0" xmlns:smil20="http://www.w3.org/2001/SMIL20/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xmlns:smil20lang="http://www.w3.org/2001/SMIL20/Language" xmlns:pbase="http://www.opengis.net/citygml/profiles/base/2.0" xmlns:core="http://www.opengis.net/citygml/2.0" xmlns:grp="http://www.opengis.net/citygml/cityobjectgroup/2.0" xsi:schemaLocation="https://www.geospatial.jp/iur/uro/3.1 ../../schemas/iur/uro/3.1/urbanObject.xsd http://www.opengis.net/citygml/2.0 http://schemas.opengis.net/citygml/2.0/cityGMLBase.xsd http://www.opengis.net/citygml/landuse/2.0 http://schemas.opengis.net/citygml/landuse/2.0/landUse.xsd http://www.opengis.net/citygml/building/2.0 http://schemas.opengis.net/citygml/building/2.0/building.xsd http://www.opengis.net/citygml/transportation/2.0 http://schemas.opengis.net/citygml/transportation/2.0/transportation.xsd http://www.opengis.net/citygml/generics/2.0 http://schemas.opengis.net/citygml/generics/2.0/generics.xsd http://www.opengis.net/citygml/cityobjectgroup/2.0 http://schemas.opengis.net/citygml/cityobjectgroup/2.0/cityObjectGroup.xsd http://www.opengis.net/gml http://schemas.opengis.net/gml/3.1.1/base/gml.xsd http://www.opengis.net/citygml/appearance/2.0 http://schemas.opengis.net/citygml/appearance/2.0/appearance.xsd">
	<gml:boundedBy>
		<gml:Envelope srsName="http://www.opengis.net/def/crs/EPSG/0/6697" srsDimension="3">
			<gml:lowerCorner>35.458981040061694 139.61388822436427 -5.40387417</gml:lowerCorner>
			<gml:upperCorner>35.46652021723128 139.6250504600989 24.44694595</gml:upperCorner>
		</gml:Envelope>
	</gml:boundedBy>
	<core:cityObjectMember>
		<brid:Bridge gml:id="brid_c3317e30-cbd6-4ab4-ad3b-b07ad3a1b72f">
			<core:creationDate>2024-03-22</core:creationDate>
			<brid:class codeSpace="../../codelists/Bridge_class.xml">01</brid:class>
			<brid:function codeSpace="../../codelists/Bridge_function.xml">07</brid:function>
			<brid:yearOfConstruction>0001</brid:yearOfConstruction>
			<brid:lod2Solid>
				<gml:Solid>
					<gml:exterior>
						<gml:CompositeSurface>
							<gml:surfaceMember xlink:href="#poly_YHHW0072_p521_0"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0072_p521_1"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0072_p521_2"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0072_p521_3"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0072_p521_4"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0072_p520_0"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0072_p520_1"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0072_p520_2"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0072_p520_3"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0072_p520_4"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0072_p520_5"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0072_p520_6"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0072_p520_7"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0072_p520_8"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0072_p520_9"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0072_p519_0"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0072_p519_1"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0072_p519_2"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0072_p519_3"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0072_p519_4"/>
							<gml:surfaceMember xlink:href="#poly_YHHW0072_b_0"/>
						</gml:CompositeSurface>
					</gml:exterior>
				</gml:Solid>
			</brid:lod2Solid>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0072_p521_2">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0072_p521_2">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0072_p521_2">
											<gml:posList>35.464435579028496 139.6193750150817 2.40015972 35.464429849059236 139.61935670964687 3.73590909 35.46442986094319 139.61935668872007 0.64218786 35.464435585785786 139.61937500319024 0.64143812 35.464435579028496 139.6193750150817 2.40015972</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0072_p521_3">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0072_p521_3">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0072_p521_3">
											<gml:posList>35.464472165740005 139.61933470152692 3.73913864 35.46447623627078 139.61935350647173 2.40328276 35.46447624303941 139.61935349457428 0.64456116 35.46447217764448 139.61933468058933 0.64541741 35.464472165740005 139.61933470152692 3.73913864</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0072_p521_4">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0072_p521_4">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0072_p521_4">
											<gml:posList>35.46447623627078 139.61935350647173 2.40328276 35.464435579028496 139.6193750150817 2.40015972 35.464435585785786 139.61937500319024 0.64143812 35.46447624303941 139.61935349457428 0.64456116 35.46447623627078 139.61935350647173 2.40328276</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0072_p520_3">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0072_p520_3">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0072_p520_3">
											<gml:posList>35.46446662837504 139.61931345948554 3.74006113 35.464472165740005 139.61933470152692 3.73913864 35.46447217764448 139.61933468058933 0.64541741 35.46446664027679 139.61931343853763 0.6463399 35.46446662837504 139.61931345948554 3.74006113</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0072_p520_4">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0072_p520_4">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0072_p520_4">
											<gml:posList>35.46442986094319 139.61935668872007 0.64218786 35.464429849059236 139.61935670964687 3.73590909 35.46442681009897 139.6193481287776 3.73624368 35.46442681586023 139.6193481186291 2.23624409 35.464426821981384 139.61934810784663 0.64252245 35.46442986094319 139.61935668872007 0.64218786</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0072_p520_5">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0072_p520_5">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0072_p520_5">
											<gml:posList>35.464456404723876 139.61930422311707 0.64636944 35.464426821981384 139.61934810784663 0.64252245 35.46442681586023 139.6193481186291 2.23624409 35.46445639859527 139.61930423391058 2.24009109 35.464456404723876 139.61930422311707 0.64636944</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0072_p520_8">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0072_p520_8">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0072_p520_8">
											<gml:posList>35.46439385319894 139.61931895391552 6.76631423 35.46439385895235 139.6193189437602 5.26631465 35.46442681586023 139.6193481186291 2.23624409 35.46442681009897 139.6193481287776 3.73624368 35.46439385319894 139.61931895391552 6.76631423</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0072_p520_9">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0072_p520_9">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0072_p520_9">
											<gml:posList>35.464456404723876 139.61930422311707 0.64636944 35.46445639859527 139.61930423391058 2.24009109 35.464422355964466 139.61927366988647 5.27018765 35.464422350204316 139.61927368005237 6.77018723 35.46445639282708 139.61930424406935 3.74009067 35.46446662837504 139.61931345948554 3.74006113 35.46446664027679 139.61931343853763 0.6463399 35.464456404723876 139.61930422311707 0.64636944</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0072_p519_2">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0072_p519_2">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0072_p519_2">
											<gml:posList>35.464141285825995 139.61908146123008 5.26773434 35.46439385895235 139.6193189437602 5.26631465 35.46439385319894 139.61931895391552 6.76631423 35.4641412801322 139.61908147144106 6.76773392 35.464141285825995 139.61908146123008 5.26773434</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0072_p519_3">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0072_p519_3">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0072_p519_3">
											<gml:posList>35.464141285825995 139.61908146123008 5.26773434 35.4641412801322 139.61908147144106 6.76773392 35.46416977693842 139.6190361974934 6.77160686 35.46416978263895 139.61903618727163 5.27160728 35.464141285825995 139.61908146123008 5.26773434</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:WallSurface gml:id="wall_YHHW0072_p519_4">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0072_p519_4">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0072_p519_4">
											<gml:posList>35.464422355964466 139.61927366988647 5.27018765 35.46416978263895 139.61903618727163 5.27160728 35.46416977693842 139.6190361974934 6.77160686 35.464422350204316 139.61927368005237 6.77018723 35.464422355964466 139.61927366988647 5.27018765</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:WallSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:OuterCeilingSurface gml:id="ceil_YHHW0072_p520_6">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0072_p520_6">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0072_p520_6">
											<gml:posList>35.46445639859527 139.61930423391058 2.24009109 35.46442681586023 139.6193481186291 2.23624409 35.464422355964466 139.61927366988647 5.27018765 35.46445639859527 139.61930423391058 2.24009109</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:OuterCeilingSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:OuterCeilingSurface gml:id="ceil_YHHW0072_p520_7">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0072_p520_7">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0072_p520_7">
											<gml:posList>35.46442681586023 139.6193481186291 2.23624409 35.46439385895235 139.6193189437602 5.26631465 35.464422355964466 139.61927366988647 5.27018765 35.46442681586023 139.6193481186291 2.23624409</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:OuterCeilingSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:OuterCeilingSurface gml:id="ceil_YHHW0072_p519_1">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0072_p519_1">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0072_p519_1">
											<gml:posList>35.46416978263895 139.61903618727163 5.27160728 35.464422355964466 139.61927366988647 5.27018765 35.46439385895235 139.6193189437602 5.26631465 35.464141285825995 139.61908146123008 5.26773434 35.46416978263895 139.61903618727163 5.27160728</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:OuterCeilingSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:OuterFloorSurface gml:id="floor_YHHW0072_p521_0">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0072_p521_0">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0072_p521_0">
											<gml:posList>35.46447623627078 139.61935350647173 2.40328276 35.464472165740005 139.61933470152692 3.73913864 35.464429849059236 139.61935670964687 3.73590909 35.46447623627078 139.61935350647173 2.40328276</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:OuterFloorSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:OuterFloorSurface gml:id="floor_YHHW0072_p521_1">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0072_p521_1">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0072_p521_1">
											<gml:posList>35.464429849059236 139.61935670964687 3.73590909 35.464435579028496 139.6193750150817 2.40015972 35.46447623627078 139.61935350647173 2.40328276 35.464429849059236 139.61935670964687 3.73590909</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:OuterFloorSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:OuterFloorSurface gml:id="floor_YHHW0072_p520_0">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0072_p520_0">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0072_p520_0">
											<gml:posList>35.46446662837504 139.61931345948554 3.74006113 35.46445639282708 139.61930424406935 3.74009067 35.46442681009897 139.6193481287776 3.73624368 35.464429849059236 139.61935670964687 3.73590909 35.464472165740005 139.61933470152692 3.73913864 35.46446662837504 139.61931345948554 3.74006113</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:OuterFloorSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:OuterFloorSurface gml:id="floor_YHHW0072_p520_1">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0072_p520_1">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0072_p520_1">
											<gml:posList>35.46442681009897 139.6193481287776 3.73624368 35.46445639282708 139.61930424406935 3.74009067 35.464422350204316 139.61927368005237 6.77018723 35.46442681009897 139.6193481287776 3.73624368</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:OuterFloorSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:OuterFloorSurface gml:id="floor_YHHW0072_p520_2">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0072_p520_2">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0072_p520_2">
											<gml:posList>35.464422350204316 139.61927368005237 6.77018723 35.46439385319894 139.61931895391552 6.76631423 35.46442681009897 139.6193481287776 3.73624368 35.464422350204316 139.61927368005237 6.77018723</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:OuterFloorSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:OuterFloorSurface gml:id="floor_YHHW0072_p519_0">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0072_p519_0">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0072_p519_0">
											<gml:posList>35.4641412801322 139.61908147144106 6.76773392 35.46439385319894 139.61931895391552 6.76631423 35.464422350204316 139.61927368005237 6.77018723 35.46416977693842 139.6190361974934 6.77160686 35.4641412801322 139.61908147144106 6.76773392</gml:posList>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:MultiSurface>
					</brid:lod2MultiSurface>
				</brid:OuterFloorSurface>
			</brid:boundedBy>
			<brid:boundedBy>
				<brid:GroundSurface gml:id="gnd_YHHW0072_b_0">
					<brid:lod2MultiSurface>
						<gml:MultiSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="poly_YHHW0072_b_0">
									<gml:exterior>
										<gml:LinearRing gml:id="line_YHHW0072_b_0">
											<gml:posList>35.46446664027679 139.61931343853763 0.6463399 35.46447217764448 139.61933468058933 0.64541741 35.46447624303941 139.61935349457428 0.64456116 35.464435585785786 139.61937500319024 0.64143812 35.46442986094319 139.61935668872007 0.64218786 35.464426821981384 139.61934810784663 0.64252245 35.464456404723876 139.61930422311707 0.64636944 35.46446664027679 139.61931343853763 0.6463399</gml:posList>
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
