<?xml version="1.0" encoding="utf-8"?>
<!-- hand-authored CityParquet test fixture (W-M5b: materials + textures). -->
<!-- A Building with one lod2Solid tetrahedron of four inline polygons p0..p3 -->
<!-- (each exterior ring has a gml:id p<n>_r0). Theme "visual": X3DMaterials -->
<!-- red -> {p0,p1}, green -> {p2}; a ParameterizedTexture textures ring p0_r0. -->
<!-- The texture coords are closed (last pair == first); the reader drops it. -->
<CityModel xmlns:xlink="http://www.w3.org/1999/xlink"
           xmlns:bldg="http://www.opengis.net/citygml/building/2.0"
           xmlns:app="http://www.opengis.net/citygml/appearance/2.0"
           xmlns:gml="http://www.opengis.net/gml"
           xmlns="http://www.opengis.net/citygml/2.0">
	<cityObjectMember>
		<bldg:Building gml:id="BA">
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
			<app:appearance>
				<app:Appearance>
					<app:theme>visual</app:theme>
					<app:surfaceDataMember>
						<app:X3DMaterial>
							<gml:name>red</gml:name>
							<app:diffuseColor>1.0 0.0 0.0</app:diffuseColor>
							<app:target>#p0</app:target>
							<app:target>#p1</app:target>
						</app:X3DMaterial>
					</app:surfaceDataMember>
					<app:surfaceDataMember>
						<app:X3DMaterial>
							<gml:name>green</gml:name>
							<app:diffuseColor>0.0 1.0 0.0</app:diffuseColor>
							<app:target>#p2</app:target>
						</app:X3DMaterial>
					</app:surfaceDataMember>
					<app:surfaceDataMember>
						<app:ParameterizedTexture>
							<app:imageURI>textures/wall.jpg</app:imageURI>
							<app:mimeType>image/jpeg</app:mimeType>
							<app:textureType>unknown</app:textureType>
							<app:wrapMode>wrap</app:wrapMode>
							<app:borderColor>0.0 0.0 0.0 1.0</app:borderColor>
							<app:target uri="#p0">
								<app:TexCoordList>
									<app:textureCoordinates ring="#p0_r0">0.0 0.0 1.0 0.0 0.0 1.0 0.0 0.0</app:textureCoordinates>
								</app:TexCoordList>
							</app:target>
						</app:ParameterizedTexture>
					</app:surfaceDataMember>
				</app:Appearance>
			</app:appearance>
		</bldg:Building>
	</cityObjectMember>
</CityModel>
