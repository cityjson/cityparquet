<?xml version="1.0" encoding="utf-8"?>
<!-- hand-authored CityParquet test fixture (CG-3: CityModel-level appearance). -->
<!-- The app:appearanceMember (conformant CityModel-level global appearance)    -->
<!-- is a child of CityModel, sibling of cityObjectMember, and appears AFTER     -->
<!-- the building, so only a two-pass reader -->
<!-- can apply it. Its X3DMaterial (red) targets solid face #p0; its texture    -->
<!-- targets ring #p1_r0. A reader that only reads appearance INSIDE a Building  -->
<!-- assigns no material/texture at all. -->
<CityModel xmlns:xlink="http://www.w3.org/1999/xlink"
           xmlns:bldg="http://www.opengis.net/citygml/building/2.0"
           xmlns:app="http://www.opengis.net/citygml/appearance/2.0"
           xmlns:gml="http://www.opengis.net/gml"
           xmlns="http://www.opengis.net/citygml/2.0">
	<cityObjectMember>
		<bldg:Building gml:id="BM">
			<bldg:lod2Solid>
				<gml:Solid>
					<gml:exterior>
						<gml:CompositeSurface>
							<gml:surfaceMember>
								<gml:Polygon gml:id="p0">
									<gml:exterior>
										<gml:LinearRing gml:id="p0_r0">
											<gml:pos>0.0 0.0 0.0</gml:pos>
											<gml:pos>10.0 0.0 0.0</gml:pos>
											<gml:pos>0.0 10.0 0.0</gml:pos>
											<gml:pos>0.0 0.0 0.0</gml:pos>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="p1">
									<gml:exterior>
										<gml:LinearRing gml:id="p1_r0">
											<gml:pos>0.0 0.0 0.0</gml:pos>
											<gml:pos>0.0 0.0 10.0</gml:pos>
											<gml:pos>10.0 0.0 0.0</gml:pos>
											<gml:pos>0.0 0.0 0.0</gml:pos>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="p2">
									<gml:exterior>
										<gml:LinearRing gml:id="p2_r0">
											<gml:pos>10.0 0.0 0.0</gml:pos>
											<gml:pos>0.0 0.0 10.0</gml:pos>
											<gml:pos>0.0 10.0 0.0</gml:pos>
											<gml:pos>10.0 0.0 0.0</gml:pos>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
							<gml:surfaceMember>
								<gml:Polygon gml:id="p3">
									<gml:exterior>
										<gml:LinearRing gml:id="p3_r0">
											<gml:pos>0.0 10.0 0.0</gml:pos>
											<gml:pos>0.0 0.0 10.0</gml:pos>
											<gml:pos>0.0 0.0 0.0</gml:pos>
											<gml:pos>0.0 10.0 0.0</gml:pos>
										</gml:LinearRing>
									</gml:exterior>
								</gml:Polygon>
							</gml:surfaceMember>
						</gml:CompositeSurface>
					</gml:exterior>
				</gml:Solid>
			</bldg:lod2Solid>
		</bldg:Building>
	</cityObjectMember>
	<app:appearanceMember>
		<app:Appearance>
			<app:theme>visual</app:theme>
			<app:surfaceDataMember>
				<app:X3DMaterial>
					<gml:name>red</gml:name>
					<app:diffuseColor>1.0 0.0 0.0</app:diffuseColor>
					<app:target>#p0</app:target>
				</app:X3DMaterial>
			</app:surfaceDataMember>
			<app:surfaceDataMember>
				<app:ParameterizedTexture>
					<app:imageURI>textures/wall.jpg</app:imageURI>
					<app:mimeType>image/jpeg</app:mimeType>
					<app:target uri="#p1">
						<app:TexCoordList>
							<app:textureCoordinates ring="#p1_r0">0.0 0.0 1.0 0.0 0.0 1.0 0.0 0.0</app:textureCoordinates>
						</app:TexCoordList>
					</app:target>
				</app:ParameterizedTexture>
			</app:surfaceDataMember>
		</app:Appearance>
	</app:appearanceMember>
</CityModel>
